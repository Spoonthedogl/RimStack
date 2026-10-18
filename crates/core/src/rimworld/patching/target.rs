use super::DefKey;
use super::document::Document;
use super::xpath::{self, Test};
use std::collections::BTreeSet;

/// What an XPath is known to reach among the top-level defs: the union of its sides.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    sides: Vec<Side>,
}

/// One side of a union; a `None` is any.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Side {
    kind: Option<String>,
    names: Option<Names>,
}

/// What a side is narrowed to. A def with a `defName` is keyed by it, which its `Name` and its `ParentName`
/// do not tell, so these wait for `Target::resolve`; until then they stand for every def of the kind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Names {
    def_names: BTreeSet<String>,
    attributes: BTreeSet<String>,
    parents: BTreeSet<String>,
}

impl Names {
    fn waits(&self) -> bool {
        !self.attributes.is_empty() || !self.parents.is_empty()
    }
}

impl Target {
    pub fn any() -> Target {
        Target {
            sides: vec![Side::any()],
        }
    }

    pub fn of_xpath(xpath: &str) -> Target {
        let path = xpath.trim();
        if leaves_the_def(path) {
            return Target::any();
        }
        Target {
            sides: sides_of(path),
        }
    }

    pub fn covers(&self, def: &DefKey) -> bool {
        self.sides.iter().any(|side| side.covers(def))
    }

    pub fn overlaps(&self, other: &Target) -> bool {
        self.sides
            .iter()
            .any(|one| other.sides.iter().any(|two| one.overlaps(two)))
    }

    /// The kind of a target that is one side narrowed to it.
    pub fn kind(&self) -> Option<&str> {
        match self.sides.as_slice() {
            [side] => side.kind.as_deref(),
            _ => None,
        }
    }

    /// The names of a target that is one side narrowed to them.
    pub fn names(&self) -> Option<&BTreeSet<String>> {
        match self.sides.as_slice() {
            [side] => side.names.as_ref().map(|names| &names.def_names),
            _ => None,
        }
    }

    /// Whether `resolve` would narrow it.
    pub fn waits_for_the_document(&self) -> bool {
        self.sides
            .iter()
            .any(|side| side.names.as_ref().is_some_and(Names::waits))
    }

    /// The defs of `document` that its `Name` and `ParentName` tests name, put among the names it is narrowed to.
    pub(super) fn resolve(&self, document: &Document) -> Target {
        Target {
            sides: self
                .sides
                .iter()
                .map(|side| side.resolve(document))
                .collect(),
        }
    }

    /// Every side is narrowed to a kind.
    pub fn has_kind(&self) -> bool {
        self.sides.iter().all(|side| side.kind.is_some())
    }

    /// Every side is narrowed to names.
    pub fn has_names(&self) -> bool {
        self.sides.iter().all(|side| side.names.is_some())
    }
}

impl Side {
    fn any() -> Side {
        Side {
            kind: None,
            names: None,
        }
    }

    fn of_path(path: &str) -> Side {
        if path.starts_with("//") {
            return Side::any();
        }
        let Some(below) = xpath::below_defs(path.trim_start_matches('/')) else {
            return Side::any();
        };
        let end = below.find(['[', '/']).unwrap_or(below.len());
        let kind = below[..end].trim_end();
        if kind.is_empty()
            || kind.contains(['*', '(', '@', ':'])
            || kind.contains(char::is_whitespace)
        {
            return Side::any();
        }
        Side {
            kind: Some(kind.to_string()),
            names: name_predicate(below[end..].trim_start()),
        }
    }

    fn resolve(&self, document: &Document) -> Side {
        let (Some(kind), Some(names)) = (&self.kind, &self.names) else {
            return self.clone();
        };
        let mut def_names = names.def_names.clone();
        for attribute in &names.attributes {
            def_names.extend(
                document
                    .keys_named(kind, attribute)
                    .into_iter()
                    .map(|key| key.name),
            );
        }
        for parent in &names.parents {
            let derived = document.keys_derived(kind, parent);
            def_names.extend(derived.into_iter().map(|key| key.name));
        }
        Side {
            kind: self.kind.clone(),
            names: Some(Names {
                def_names,
                ..Names::default()
            }),
        }
    }

    fn covers(&self, def: &DefKey) -> bool {
        self.kind.as_ref().is_none_or(|kind| *kind == def.kind)
            && self
                .names
                .as_ref()
                .is_none_or(|names| names.waits() || names.def_names.contains(&def.name))
    }

    fn overlaps(&self, other: &Side) -> bool {
        let kinds = self.kind.is_none() || other.kind.is_none() || self.kind == other.kind;
        let names = match (&self.names, &other.names) {
            (Some(a), Some(b)) => a.waits() || b.waits() || !a.def_names.is_disjoint(&b.def_names),
            _ => true,
        };
        kinds && names
    }
}

fn sides_of(path: &str) -> Vec<Side> {
    let parts = split_union(path);
    if parts.len() > 1 {
        return parts.into_iter().flat_map(sides_of).collect();
    }
    match grouped(path) {
        Some((inside, rest)) => split_union(inside)
            .into_iter()
            .flat_map(|part| sides_of(&format!("{part}{rest}")))
            .collect(),
        None => vec![Side::of_path(path)],
    }
}

/// Where a `|` stands outside quotes, parentheses and brackets.
fn split_union(path: &str) -> Vec<&str> {
    let (mut parts, mut start, mut quote, mut depth) = (Vec::new(), 0, None, 0usize);
    for (at, c) in path.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth = depth.saturating_sub(1),
            (None, '|') if depth == 0 => {
                parts.push(path[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(path[start..].trim());
    parts
}

/// `(inside)rest`, when the parenthesis that opens the path closes before its end.
fn grouped(path: &str) -> Option<(&str, &str)> {
    let inside = path.strip_prefix('(')?;
    let (mut quote, mut depth) = (None, 0usize);
    for (at, c) in inside.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '(') => depth += 1,
            (None, ')') if depth == 0 => return Some((&inside[..at], &inside[at + 1..])),
            (None, ')') => depth -= 1,
            _ => {}
        }
    }
    None
}

fn leaves_the_def(path: &str) -> bool {
    const AXES: [&str; 5] = ["parent::", "ancestor", "following", "preceding", ".."];
    AXES.iter().any(|axis| path.contains(axis))
}

fn name_predicate(rest: &str) -> Option<Names> {
    let mut names = Names::default();
    for test in xpath::name_tests(rest)?.tests {
        match test {
            Test::DefName(name) => names.def_names.insert(name),
            Test::Name(name) => names.attributes.insert(name),
            Test::ParentName(name) => names.parents.insert(name),
        };
    }
    Some(names)
}

#[cfg(test)]
mod tests;
