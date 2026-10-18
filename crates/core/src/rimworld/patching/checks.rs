use super::document::Document;
use super::inheritance::{Elem, Source, resolve};
use super::types::Types;
use super::{Active, DefKey, Failure};
use crate::rimworld::dotnet::FieldType;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

/// The failures of the defs the game would make, after inheritance. `rank` gives each place its position
/// in the load order.
pub(super) fn find(
    doc: &Document,
    rank: &[Option<usize>],
    active: &Active,
    types: &Types,
) -> Vec<Failure> {
    let tops = doc.tops();
    let (sources, makes_def): (Vec<_>, Vec<_>) = tops
        .into_par_iter()
        .filter_map(|top| {
            let required = top
                .node
                .get_attribute("MayRequire")
                .filter(|v| !v.is_empty());
            if required.is_some_and(|list| !active.all_ids(&list)) {
                return None;
            }
            let any = top
                .node
                .get_attribute("MayRequireAnyOf")
                .filter(|v| !v.is_empty());
            let source = Source {
                key: top.key,
                rank: top
                    .place
                    .and_then(|place| rank.get(place).copied().flatten()),
                elem: Elem::of(top.node, active),
            };
            Some((source, any.is_none_or(|list| active.any_id(&list))))
        })
        .unzip();
    let (resolved, duplicates) = resolve(&sources);
    let mut failures = Vec::new();
    let mut seen = HashSet::new();
    let mut duplicate = |def: &DefKey, element: &str, failures: &mut Vec<Failure>| {
        if seen.insert((def.clone(), element.to_string())) {
            failures.push(Failure::Duplicate {
                def: def.clone(),
                element: element.to_string(),
            });
        }
    };
    for (def, element) in &duplicates {
        duplicate(def, element, &mut failures);
    }
    let mut last: HashMap<&DefKey, usize> = HashMap::new();
    for (at, source) in sources.iter().enumerate() {
        let real = makes_def[at] && !is_abstract(&source.elem);
        if let (true, Some(key)) = (real, &source.key) {
            last.insert(key, at);
        }
    }
    let walked: Vec<Vec<Failure>> = (0..sources.len())
        .into_par_iter()
        .map(|at| {
            let source = &sources[at];
            let (Some(key), Some(elem)) = (&source.key, resolved[at].get(&source.elem)) else {
                return Vec::new();
            };
            if last.get(key) != Some(&at) {
                return Vec::new();
            }
            let mut found = Vec::new();
            if matches!(elem.name.as_str(), "ThingDef" | "AbilityDef") {
                for stats in elem.elements().filter(|e| e.name == "statBases") {
                    let mut names = HashSet::new();
                    for stat in stats.elements() {
                        if !names.insert(stat.name.as_str()) {
                            found.push(Failure::Duplicate {
                                def: key.clone(),
                                element: stat.name.clone(),
                            });
                        }
                    }
                }
            }
            let class = elem.attrs.get("Class").map(String::as_str);
            if let Some(kind) = types.def_type(&elem.name, class) {
                Walk { types, key }.fields(elem, kind, &mut found);
            }
            found
        })
        .collect();
    for failure in walked.into_iter().flatten() {
        match failure {
            Failure::Duplicate { def, element } => duplicate(&def, &element, &mut failures),
            other => failures.push(other),
        }
    }
    failures
}

fn is_abstract(elem: &Elem) -> bool {
    elem.attrs
        .get("Abstract")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

struct Walk<'a> {
    types: &'a Types,
    key: &'a DefKey,
}

impl Walk<'_> {
    fn fields(&self, elem: &Elem, of: &str, out: &mut Vec<Failure>) {
        for child in elem.elements().filter(|c| c.name != "li") {
            match self.types.field(of, &child.name) {
                Some(FieldType::ListOf(item)) => self.list(child, item, out),
                Some(FieldType::Named(named)) => {
                    self.fields(child, &self.class_or(child, named), out)
                }
                _ => {}
            }
        }
    }

    fn list(&self, field: &Elem, item: &str, out: &mut Vec<Failure>) {
        if !self.types.is_def(item) {
            for li in field.elements() {
                self.fields(li, &self.class_or(li, item), out);
            }
            return;
        }
        let mut seen = HashSet::new();
        let mut reported = HashSet::new();
        for li in field.elements().filter(|l| l.name == "li") {
            let Some(entry) = li.text().map(str::trim).filter(|t| !t.is_empty()) else {
                continue;
            };
            if !seen.insert(entry) && reported.insert(entry) {
                out.push(Failure::Twice {
                    def: self.key.clone(),
                    field: field.name.clone(),
                    entry: entry.to_string(),
                });
            }
        }
    }

    fn class_or(&self, elem: &Elem, field_type: &str) -> String {
        match elem.attrs.get("Class") {
            Some(class) => self
                .types
                .def_type("", Some(class))
                .unwrap_or(class)
                .to_string(),
            None => field_type.to_string(),
        }
    }
}
