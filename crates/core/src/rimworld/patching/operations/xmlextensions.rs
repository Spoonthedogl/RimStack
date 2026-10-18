//! XmlExtensions' patch operations (workshop 2574315206), resolved while reading: the stored values
//! and the active mods are known then, so a branch the game would not take is never read, and a
//! `{key}` substitution is made on the text of `apply` before it is read as operations.

use super::read::{Frag, Operation, Reading, Work, child, frags, operation, text_of, unknown};
use super::safe::{Compare, Kind, Safe};
use crate::rimworld::patching::ModFiles;
use crate::rimworld::patching::document::files_in_game_order;
use libxml::parser::{Parser, ParserOptions};
use libxml::tree::{Node, NodeType};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::OnceLock;

const CLASS: &str = "XmlExtensions.";
const DEFAULT_BRACKETS: &str = "{}";

/// The `XmlExtensions.PatchDef`s of the mods' `Defs`, by `Name`, read when the first `ApplyPatch` asks.
pub(super) struct PatchDefs<'a> {
    mods: &'a [ModFiles],
    found: OnceLock<HashMap<String, PatchDef>>,
}

struct PatchDef {
    parameters: Vec<String>,
    apply: Option<String>,
    brackets: String,
}

impl<'a> PatchDefs<'a> {
    pub fn new(mods: &'a [ModFiles]) -> Self {
        PatchDefs {
            mods,
            found: OnceLock::new(),
        }
    }

    fn get(&self, name: &str) -> Option<&PatchDef> {
        self.found.get_or_init(|| scan(self.mods)).get(name)
    }
}

fn scan(mods: &[ModFiles]) -> HashMap<String, PatchDef> {
    let files: Vec<_> = mods
        .iter()
        .flat_map(|files| files_in_game_order(files, "Defs"))
        .collect();
    let found: Vec<(String, PatchDef)> = files
        .par_iter()
        .flat_map_iter(|path| {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            if text.contains("XmlExtensions.PatchDef") {
                defs_in(&text)
            } else {
                Vec::new()
            }
        })
        .collect();
    let mut by_name = HashMap::new();
    for (name, def) in found {
        by_name.entry(name).or_insert(def);
    }
    by_name
}

fn defs_in(text: &str) -> Vec<(String, PatchDef)> {
    let Ok(tree) = Parser::default().parse_string(text) else {
        return Vec::new();
    };
    let Some(root) = tree.get_root_element() else {
        return Vec::new();
    };
    root.get_child_elements()
        .iter()
        .filter(|def| def.get_name() == "XmlExtensions.PatchDef")
        .filter_map(|def| {
            let name = def.get_attribute("Name")?;
            let def = PatchDef {
                parameters: strings(def, "parameters").unwrap_or_default(),
                apply: child(def, "apply").map(|apply| outer_xml(&apply)),
                brackets: text_of(def, "brackets").unwrap_or_else(|| DEFAULT_BRACKETS.into()),
            };
            Some((name, def))
        })
        .collect()
}

pub(super) fn read(class: &str, node: &Node, reading: &Reading) -> Option<Work> {
    let name = class.strip_prefix(CLASS)?;
    let xpath = text_of(node, "xpath");
    let single = flag(node, "selectSingleNode");
    let value = || child(node, "value").map(|v| frags(&v.get_child_nodes()));
    let work = match name {
        "PatchOperationAdd" => Work::Add {
            xpath,
            value: value(),
            prepend: text_of(node, "order").as_deref() == Some("Prepend"),
            single,
        },
        "PatchOperationInsert" => Work::Insert {
            xpath,
            value: value(),
            prepend: text_of(node, "order").as_deref() != Some("Append"),
            single,
        },
        "PatchOperationReplace" => Work::Replace {
            xpath,
            value: value(),
            single,
        },
        "PatchOperationRemove" => Work::Remove { xpath, single },
        "PatchOperationSetName" => Work::SetName {
            xpath,
            name: text_of(node, "name"),
            single,
        },
        "PatchOperationSafeRemove" => Work::SafeRemove { xpath },
        "PatchOperationSafeAdd" => safe(Kind::Add, node, xpath, single, value()),
        "PatchOperationSafeReplace" => safe(Kind::Replace, node, xpath, single, value()),
        "PatchOperationSafeAddOrReplace" => safe(Kind::AddOrReplace, node, xpath, single, value()),
        "PatchOperationAddOrReplace" => safe(Kind::Flat, node, xpath, single, value()),
        "Sequence" => nested(operations(node, "apply", reading)),
        "Conditional" => Work::Branch {
            xpath,
            yes: operations(node, "caseTrue", reading),
            no: operations(node, "caseFalse", reading),
        },
        "FindMod" => find_mod(node, reading),
        "OptionalPatch" => optional(node, reading),
        "UseSetting" => use_setting(node, reading),
        "UseSettings" => use_settings(node, reading),
        "ApplyPatch" => apply_patch(node, reading),
        _ => return None,
    };
    let elsewhere = child(node, "xmlDoc").is_some() || flag(node, "requiresDelay");
    Some(if elsewhere { unknown(node) } else { work })
}

pub(super) fn nested(operations: Vec<Operation>) -> Work {
    Work::Nested {
        operations,
        listed: None,
    }
}

fn flag(node: &Node, name: &str) -> bool {
    text_of(node, name).is_some_and(|t| t.eq_ignore_ascii_case("true"))
}

fn strings(node: &Node, name: &str) -> Option<Vec<String>> {
    let list = child(node, name)?;
    Some(
        list.get_child_elements()
            .iter()
            .map(|li| li.get_content().trim().to_string())
            .collect(),
    )
}

/// The operations in a container such as `apply` or `caseTrue`; a missing or empty one has none.
fn operations(node: &Node, name: &str, reading: &Reading) -> Vec<Operation> {
    let list = child(node, name).map(|c| c.get_child_elements());
    let list = list.unwrap_or_default();
    list.iter().map(|li| operation(li, reading)).collect()
}

fn safe(
    kind: Kind,
    node: &Node,
    xpath: Option<String>,
    single: bool,
    value: Option<Vec<Frag>>,
) -> Work {
    let compare = match text_of(node, "compare").as_deref() {
        Some("InnerText") => Compare::InnerText,
        Some("Both") => Compare::Both,
        _ => Compare::Name,
    };
    let depth = text_of(node, "safetyDepth").and_then(|d| d.parse::<i64>().ok());
    Work::Safe(Safe {
        kind,
        xpath,
        single,
        value,
        depth: depth.filter(|d| *d >= 0).map_or(99999, |d| d as usize),
        attributes: flag(node, "checkAttributes"),
        compare,
    })
}

fn find_mod(node: &Node, reading: &Reading) -> Work {
    let Some(mods) = strings(node, "mods") else {
        return Work::Fails;
    };
    let by_id = flag(node, "packageId");
    let logic = text_of(node, "logic")
        .unwrap_or_else(|| "or".into())
        .to_lowercase();
    let active = reading.active;
    let mut found = false;
    for name in &mods {
        let here = if by_id {
            active.has_id_like(name)
        } else {
            active.has_name_like(name)
        };
        if here {
            found = true;
            if logic == "or" {
                break;
            }
        } else {
            found = false;
            if logic == "and" {
                break;
            }
        }
    }
    let case = if found { "caseTrue" } else { "caseFalse" };
    nested(operations(node, case, reading))
}

/// `bool.Parse`: any case, spaces around.
fn boolean(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// What is stored for `modId;key`, else the operation's own default.
fn setting(reading: &Reading, mod_id: &str, key: &str, default: Option<&str>) -> Option<String> {
    let stored = reading.settings.xml_extensions(mod_id, key);
    stored.or(default).map(str::to_string)
}

fn optional(node: &Node, reading: &Reading) -> Work {
    let key = text_of(node, "key");
    let keys = strings(node, "keys");
    let default = text_of(node, "defaultValue");
    let defaults = strings(node, "defaultValues");
    let missing = (key.is_none() && keys.is_none()) || (default.is_none() && defaults.is_none());
    let Some(mod_id) = text_of(node, "modId").filter(|_| !missing) else {
        return Work::Fails;
    };
    let on = match (&keys, &defaults, &key) {
        (Some(keys), Some(defaults), _) if keys.len() == defaults.len() => {
            every_key(node, reading, &mod_id, keys, defaults)
        }
        (Some(_), _, _) | (None, _, None) => None,
        (None, _, Some(key)) => {
            setting(reading, &mod_id, key, default.as_deref()).and_then(|v| boolean(&v))
        }
    };
    match on {
        Some(on) => nested(operations(
            node,
            if on { "caseTrue" } else { "caseFalse" },
            reading,
        )),
        None => Work::Fails,
    }
}

/// `And` (the default) wants every key on, `Or` any; a text that is no bool throws where it is read.
fn every_key(
    node: &Node,
    reading: &Reading,
    mod_id: &str,
    keys: &[String],
    defaults: &[String],
) -> Option<bool> {
    let all = text_of(node, "logic").as_deref() != Some("Or");
    let mut flag = true;
    for (key, default) in keys.iter().zip(defaults) {
        let text = setting(reading, mod_id, key, Some(default))?;
        let on = boolean(&text)?;
        if all {
            flag &= on;
        } else if on {
            return Some(true);
        } else {
            flag = false;
        }
    }
    Some(flag)
}

fn use_setting(node: &Node, reading: &Reading) -> Work {
    let (Some(mod_id), Some(key)) = (text_of(node, "modId"), text_of(node, "key")) else {
        return Work::Fails;
    };
    let default = text_of(node, "defaultValue");
    let value = setting(reading, &mod_id, &key, default.as_deref()).unwrap_or_default();
    substituted(node, reading, &[key], &[value])
}

fn use_settings(node: &Node, reading: &Reading) -> Work {
    let Some(mod_id) = text_of(node, "modId") else {
        return Work::Fails;
    };
    let (Some(keys), Some(defaults)) = (strings(node, "keys"), strings(node, "defaultValues"))
    else {
        return Work::Fails;
    };
    if keys.len() != defaults.len() {
        return Work::Fails;
    }
    let values: Vec<String> = keys
        .iter()
        .zip(&defaults)
        .map(|(key, default)| setting(reading, &mod_id, key, Some(default)).unwrap_or_default())
        .collect();
    substituted(node, reading, &keys, &values)
}

fn substituted(node: &Node, reading: &Reading, vars: &[String], values: &[String]) -> Work {
    let brackets = text_of(node, "brackets").unwrap_or_else(|| DEFAULT_BRACKETS.into());
    let apply = child(node, "apply").map(|apply| outer_xml(&apply));
    let operations = apply
        .and_then(|apply| substitute(&apply, vars, values, &brackets))
        .and_then(|text| read_operations(&text, reading));
    operations.map_or(Work::Fails, nested)
}

fn apply_patch(node: &Node, reading: &Reading) -> Work {
    let Some(def) = text_of(node, "patchName").and_then(|name| reading.defs.get(&name)) else {
        return Work::Fails;
    };
    let arguments = strings(node, "arguments").unwrap_or_default();
    if def.parameters.len() != arguments.len() {
        return Work::Fails;
    }
    let text = def.apply.as_deref().and_then(|apply| {
        if arguments.is_empty() {
            Some(apply.to_string())
        } else {
            substitute(apply, &def.parameters, &arguments, &def.brackets)
        }
    });
    text.and_then(|text| read_operations(&text, reading))
        .map_or(Work::Fails, nested)
}

/// `SubstituteVariables`: a text replace of `<open>var<close>` for each variable in turn, on the XML as text.
fn substitute(text: &str, vars: &[String], values: &[String], brackets: &str) -> Option<String> {
    let mut chars = brackets.chars();
    let (open, close) = (chars.next()?, chars.next()?);
    let mut text = text.to_string();
    for (var, value) in vars.iter().zip(values) {
        text = text.replace(&format!("{open}{var}{close}"), value);
    }
    Some(text)
}

/// The children of the root of `text` as operations; `None` when it is no XML or nests too deep.
fn read_operations(text: &str, reading: &Reading) -> Option<Vec<Operation>> {
    let reading = reading.inside()?;
    let strict = ParserOptions {
        recover: false,
        ..ParserOptions::default()
    };
    let tree = Parser::default()
        .parse_string_with_options(text, strict)
        .ok()?;
    let root = tree.get_root_element()?;
    Some(
        root.get_child_elements()
            .iter()
            .map(|li| operation(li, &reading))
            .collect(),
    )
}

fn outer_xml(node: &Node) -> String {
    fn escape(text: &str, quote: bool) -> String {
        let text = text.replace('&', "&amp;").replace('<', "&lt;");
        if quote {
            text.replace('"', "&quot;")
        } else {
            text.replace('>', "&gt;")
        }
    }
    match node.get_type() {
        Some(NodeType::ElementNode) => {
            let mut attributes: Vec<_> = node.get_attributes().into_iter().collect();
            attributes.sort();
            let attributes: String = attributes
                .iter()
                .map(|(k, v)| format!(" {k}=\"{}\"", escape(v, true)))
                .collect();
            let inside: String = node.get_child_nodes().iter().map(outer_xml).collect();
            let name = node.get_name();
            format!("<{name}{attributes}>{inside}</{name}>")
        }
        Some(NodeType::TextNode) => escape(&node.get_content(), false),
        Some(NodeType::CDataSectionNode) => format!("<![CDATA[{}]]>", node.get_content()),
        _ => String::new(),
    }
}
