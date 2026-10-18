use super::super::{ModFiles, Target};
use super::read::{Active, Operation, Reading, Work, operation};
use super::xmlextensions::PatchDefs;
use crate::rimworld::patching::document::files_in_game_order;
use crate::rimworld::patchsettings::Settings;
use libxml::parser::{Parser, ParserOptions};
use libxml::xpath::Context;
use std::path::PathBuf;

/// What the patch files of some mods hold, for the Workshop coverage test; nothing is played.
#[derive(Default)]
pub struct Audit {
    pub files: usize,
    /// `(mod place, file, why)`.
    pub unreadable: Vec<(usize, PathBuf, String)>,
    /// `(mod place, class, played)` for each operation, nested ones included; an operation of a class
    /// RimStack does not apply counts once and its inner operations not at all.
    pub operations: Vec<(usize, String, bool)>,
    pub xpaths: Vec<XpathSeen>,
}

pub struct XpathSeen {
    pub mod_at: usize,
    pub file: PathBuf,
    pub xpath: String,
    pub target: Target,
    /// libxml2 would not evaluate it.
    pub refused: bool,
}

pub fn audit(mods: &[ModFiles], active: &Active, settings: &Settings) -> Audit {
    let defs = PatchDefs::new(mods);
    let mut found = Audit::default();
    let sample = Parser::default().parse_string("<Defs/>").ok();
    let context = sample.as_ref().and_then(|tree| Context::new(tree).ok());
    let from = sample.as_ref().and_then(|tree| tree.get_root_element());
    let refuses = |xpath: &str| match (&context, &from) {
        (Some(context), Some(from)) => context.node_evaluate_checked(xpath, from).is_err(),
        _ => false,
    };
    for (mod_at, files) in mods.iter().enumerate() {
        let reading = Reading::new(active, settings, &defs, settings.owner(mod_at));
        for path in files_in_game_order(files, "Patches") {
            found.files += 1;
            let Some(text) = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "not UTF-8".to_string()))
                .map_err(|why| found.unreadable.push((mod_at, path.clone(), why)))
                .ok()
            else {
                continue;
            };
            let strict = ParserOptions {
                recover: false,
                ..ParserOptions::default()
            };
            let tree = match Parser::default().parse_string_with_options(text, strict) {
                Ok(tree) => tree,
                Err(why) => {
                    found
                        .unreadable
                        .push((mod_at, path, format!("bad XML: {why}")));
                    continue;
                }
            };
            let Some(root) = tree.get_root_element().filter(|r| r.get_name() == "Patch") else {
                let root = tree.get_root_element().map(|r| r.get_name());
                let why = format!("wrong root <{}>", root.unwrap_or_default());
                found.unreadable.push((mod_at, path, why));
                continue;
            };
            for node in root
                .get_child_elements()
                .iter()
                .filter(|n| n.get_name() == "Operation")
            {
                let top = operation(node, &reading);
                let mut seen = Vec::new();
                walk(&top, mod_at, &mut found.operations, &mut seen);
                for xpath in seen {
                    found.xpaths.push(XpathSeen {
                        mod_at,
                        file: path.clone(),
                        target: Target::of_xpath(&xpath),
                        refused: refuses(&xpath),
                        xpath,
                    });
                }
            }
        }
    }
    found
}

fn walk(
    op: &Operation,
    mod_at: usize,
    operations: &mut Vec<(usize, String, bool)>,
    xpaths: &mut Vec<String>,
) {
    let unknown = matches!(op.work, Work::Unknown { .. } | Work::Bare);
    let class = if op.class.is_empty() {
        "(no Class)".to_string()
    } else {
        op.class.clone()
    };
    operations.push((mod_at, class, !unknown));
    let own = |xpath: &Option<String>, xpaths: &mut Vec<String>| {
        xpaths.extend(xpath.iter().cloned());
    };
    match &op.work {
        Work::Add { xpath, .. }
        | Work::Insert { xpath, .. }
        | Work::Replace { xpath, .. }
        | Work::AddModExtension { xpath, .. }
        | Work::Remove { xpath, .. }
        | Work::SetName { xpath, .. }
        | Work::Attribute { xpath, .. }
        | Work::Test { xpath } => own(xpath, xpaths),
        Work::Sequence(inner)
        | Work::Each(inner)
        | Work::Nested {
            operations: inner, ..
        } => inner
            .iter()
            .for_each(|o| walk(o, mod_at, operations, xpaths)),
        Work::Branch { xpath, yes, no } => {
            own(xpath, xpaths);
            yes.iter()
                .chain(no)
                .for_each(|o| walk(o, mod_at, operations, xpaths));
        }
        Work::SafeRemove { xpath } | Work::Slider { xpath, .. } => own(xpath, xpaths),
        Work::Safe(safe) => own(&safe.xpath, xpaths),
        Work::Fails => {}
        Work::Conditional {
            xpath,
            matched,
            unmatched,
        } => {
            own(xpath, xpaths);
            for o in [matched, unmatched].into_iter().flatten() {
                walk(o, mod_at, operations, xpaths);
            }
        }
        Work::FindMod {
            matched, unmatched, ..
        } => {
            for o in [matched, unmatched].into_iter().flatten() {
                walk(o, mod_at, operations, xpaths);
            }
        }
        Work::Unknown { aimed, .. } => xpaths.extend(aimed.iter().cloned()),
        Work::Bare => {}
    }
}
