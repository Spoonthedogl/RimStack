use super::super::ModFiles;
use super::super::document::files_in_game_order;
use crate::note;
use libxml::parser::{Parser, ParserOptions};
use libxml::tree::{Node, NodeType};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// `ids` are lowercase package ids without a `_steam` or `_copy` postfix; `names` are `About.xml` names.
#[derive(Clone, Debug, Default)]
pub struct Active {
    pub ids: HashSet<String>,
    pub names: HashSet<String>,
}

impl Active {
    fn id(&self, id: &str) -> bool {
        self.ids.contains(id.trim().to_lowercase().as_str())
    }

    pub(in crate::rimworld::patching) fn all_ids(&self, list: &str) -> bool {
        list.split(',').all(|id| self.id(id))
    }

    pub(in crate::rimworld::patching) fn any_id(&self, list: &str) -> bool {
        list.split(',').any(|id| self.id(id))
    }

    fn has_name(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

/// Every mod's patch files, in the order the game reads them.
pub struct Patches {
    pub(super) mods: Vec<Vec<PatchFile>>,
}

pub(super) struct PatchFile {
    pub path: PathBuf,
    pub operations: Vec<Operation>,
}

#[derive(Clone, Copy)]
pub(super) enum Success {
    Normal,
    Invert,
    Always,
    Never,
}

pub(super) struct Operation {
    pub class: String,
    pub success: Success,
    pub work: Work,
}

pub(super) enum Frag {
    Element {
        name: String,
        attributes: Vec<(String, String)>,
        children: Vec<Frag>,
    },
    Text(String),
}

pub(super) enum Change {
    Add,
    Set,
    Remove,
}

pub(super) enum Work {
    Add {
        xpath: Option<String>,
        value: Option<Vec<Frag>>,
        prepend: bool,
    },
    Insert {
        xpath: Option<String>,
        value: Option<Vec<Frag>>,
        prepend: bool,
    },
    Replace {
        xpath: Option<String>,
        value: Option<Vec<Frag>>,
    },
    AddModExtension {
        xpath: Option<String>,
        value: Option<Vec<Frag>>,
    },
    Remove {
        xpath: Option<String>,
    },
    SetName {
        xpath: Option<String>,
        name: Option<String>,
    },
    Attribute {
        xpath: Option<String>,
        attribute: Option<String>,
        value: String,
        change: Change,
    },
    Test {
        xpath: Option<String>,
    },
    Sequence(Vec<Operation>),
    Conditional {
        xpath: Option<String>,
        matched: Option<Box<Operation>>,
        unmatched: Option<Box<Operation>>,
    },
    FindMod {
        mods: Vec<String>,
        found: bool,
        matched: Option<Box<Operation>>,
        unmatched: Option<Box<Operation>>,
    },
    /// `aimed` is the text of every XPath element inside it, at any depth.
    Unknown {
        xpath: Option<String>,
        aimed: Vec<String>,
    },
    Bare,
}

impl Patches {
    pub(in crate::rimworld::patching) fn unknown_classes(&self) -> BTreeMap<String, usize> {
        fn count(operation: &Operation, found: &mut BTreeMap<String, usize>) {
            match &operation.work {
                Work::Unknown { .. } => *found.entry(operation.class.clone()).or_default() += 1,
                Work::Sequence(operations) => operations.iter().for_each(|o| count(o, found)),
                Work::Conditional {
                    matched, unmatched, ..
                }
                | Work::FindMod {
                    matched, unmatched, ..
                } => [matched, unmatched]
                    .into_iter()
                    .flatten()
                    .for_each(|o| count(o, found)),
                _ => {}
            }
        }
        let mut found = BTreeMap::new();
        for file in self.mods.iter().flatten() {
            file.operations.iter().for_each(|o| count(o, &mut found));
        }
        found
    }

    pub fn read(mods: &[ModFiles], active: &Active) -> Patches {
        let mods = mods
            .par_iter()
            .map(|files| {
                files_in_game_order(files, "Patches")
                    .into_par_iter()
                    .filter_map(|path| patch_file(path, active))
                    .collect()
            })
            .collect();
        Patches { mods }
    }
}

fn patch_file(path: PathBuf, active: &Active) -> Option<PatchFile> {
    let tree = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|text| {
            let strict = ParserOptions {
                recover: false,
                ..ParserOptions::default()
            };
            Parser::default()
                .parse_string_with_options(text, strict)
                .map_err(|e| e.to_string())
        });
    let tree = match tree {
        Ok(tree) => tree,
        Err(why) => {
            note!("Patch simulation skips {}: {why}", path.display());
            return None;
        }
    };
    let root = tree.get_root_element().filter(|r| r.get_name() == "Patch");
    let Some(root) = root else {
        note!("Patch simulation skips {}: no <Patch> root", path.display());
        return None;
    };
    let operations = root
        .get_child_elements()
        .iter()
        .filter(|node| expected_operation(node, &path))
        .map(|node| operation(node, active))
        .collect();
    Some(PatchFile { path, operations })
}

fn expected_operation(node: &Node, path: &Path) -> bool {
    let fine = node.get_name() == "Operation";
    if !fine {
        note!(
            "Patch simulation skips <{}> in {}: expected <Operation>",
            node.get_name(),
            path.display()
        );
    }
    fine
}

fn child(node: &Node, name: &str) -> Option<Node> {
    node.get_child_elements()
        .into_iter()
        .find(|c| c.get_name() == name)
}

fn text_of(node: &Node, name: &str) -> Option<String> {
    child(node, name).map(|c| c.get_content().trim().to_string())
}

fn branch(node: &Node, name: &str, active: &Active) -> Option<Box<Operation>> {
    child(node, name).map(|c| Box::new(operation(&c, active)))
}

pub(super) fn operation(node: &Node, active: &Active) -> Operation {
    let class = node.get_attribute("Class").unwrap_or_default();
    let xpath = text_of(node, "xpath");
    let value = || child(node, "value").map(|v| frags(&v.get_child_nodes()));
    let attribute_work = |change| Work::Attribute {
        xpath: xpath.clone(),
        attribute: text_of(node, "attribute"),
        value: text_of(node, "value").unwrap_or_default(),
        change,
    };
    let work = match class.strip_prefix("Verse.").unwrap_or(&class) {
        "PatchOperationAdd" => Work::Add {
            xpath: xpath.clone(),
            value: value(),
            prepend: text_of(node, "order").as_deref() == Some("Prepend"),
        },
        "PatchOperationInsert" => Work::Insert {
            xpath: xpath.clone(),
            value: value(),
            prepend: text_of(node, "order").as_deref() != Some("Append"),
        },
        "PatchOperationReplace" => Work::Replace {
            xpath: xpath.clone(),
            value: value(),
        },
        "PatchOperationAddModExtension" => Work::AddModExtension {
            xpath: xpath.clone(),
            value: value(),
        },
        "PatchOperationRemove" => Work::Remove {
            xpath: xpath.clone(),
        },
        "PatchOperationSetName" => Work::SetName {
            xpath: xpath.clone(),
            name: text_of(node, "name"),
        },
        "PatchOperationAttributeAdd" => attribute_work(Change::Add),
        "PatchOperationAttributeSet" => attribute_work(Change::Set),
        "PatchOperationAttributeRemove" => attribute_work(Change::Remove),
        "PatchOperationTest" => Work::Test {
            xpath: xpath.clone(),
        },
        "PatchOperationSequence" => Work::Sequence(
            child(node, "operations")
                .map(|list| {
                    list.get_child_elements()
                        .iter()
                        .filter(|li| wanted(li, active))
                        .map(|li| operation(li, active))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        "PatchOperationConditional" => Work::Conditional {
            xpath: xpath.clone(),
            matched: branch(node, "match", active),
            unmatched: branch(node, "nomatch", active),
        },
        "PatchOperationFindMod" => {
            let mods: Vec<String> = child(node, "mods")
                .map(|list| {
                    list.get_child_elements()
                        .iter()
                        .map(|li| li.get_content().trim().to_string())
                        .collect()
                })
                .unwrap_or_default();
            Work::FindMod {
                found: mods.iter().any(|name| active.has_name(name)),
                mods,
                matched: branch(node, "match", active),
                unmatched: branch(node, "nomatch", active),
            }
        }
        "" => Work::Bare,
        _ => {
            let mut aimed = Vec::new();
            xpaths_inside(node, &mut aimed);
            Work::Unknown {
                xpath: xpath.clone(),
                aimed,
            }
        }
    };
    Operation {
        class,
        success: success_of(node),
        work,
    }
}

/// An element named `xpath`, or starting or ending with `xpath`, `Xpath` or `XPath` (`xpathLocal`,
/// `sourceXPath`); `texPath` is not one.
fn xpaths_inside(node: &Node, found: &mut Vec<String>) {
    for inner in node.get_child_elements() {
        let name = inner.get_name();
        let spelled = ["xpath", "Xpath", "XPath"];
        if spelled
            .iter()
            .any(|s| name.starts_with(s) || name.ends_with(s))
        {
            found.push(inner.get_content().trim().to_string());
        }
        xpaths_inside(&inner, found);
    }
}

fn success_of(node: &Node) -> Success {
    match text_of(node, "success").as_deref() {
        Some("Invert") => Success::Invert,
        Some("Always") => Success::Always,
        Some("Never") => Success::Never,
        _ => Success::Normal,
    }
}

/// `MayRequire` needs every named mod, `MayRequireAnyOf` at least one (`DirectXmlToObject.ListFromXml`).
pub(in crate::rimworld::patching) fn wanted(node: &Node, active: &Active) -> bool {
    wanted_with(|name| node.get_attribute(name), active)
}

/// `wanted` for an element whose attributes were read already.
pub(in crate::rimworld::patching) fn wanted_by(
    attributes: &BTreeMap<String, String>,
    active: &Active,
) -> bool {
    wanted_with(|name| attributes.get(name).cloned(), active)
}

fn wanted_with(attribute: impl Fn(&str) -> Option<String>, active: &Active) -> bool {
    let all = attribute("MayRequire").filter(|v| !v.is_empty());
    if all.is_some_and(|list| !active.all_ids(&list)) {
        return false;
    }
    let any = attribute("MayRequireAnyOf").filter(|v| !v.is_empty());
    any.is_none_or(|list| active.any_id(&list))
}

fn frags(nodes: &[Node]) -> Vec<Frag> {
    let mut found = Vec::new();
    for node in nodes {
        match node.get_type() {
            Some(NodeType::ElementNode) => {
                let attributes: BTreeMap<String, String> =
                    node.get_attributes().into_iter().collect();
                found.push(Frag::Element {
                    name: node.get_name(),
                    attributes: attributes.into_iter().collect(),
                    children: frags(&node.get_child_nodes()),
                });
            }
            Some(NodeType::TextNode | NodeType::CDataSectionNode) => {
                let text = node.get_content();
                if !text.trim().is_empty() {
                    found.push(Frag::Text(text));
                }
            }
            _ => {}
        }
    }
    found
}
