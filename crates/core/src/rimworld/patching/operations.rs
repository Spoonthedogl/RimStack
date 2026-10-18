use super::document::Document;
use super::xpath::{self, Refused};
use super::{DefKey, Target};
use libxml::tree::{Node, NodeType};
use libxml::xpath::Context;
use read::{Change, Frag, Listed, Operation, Success, Work};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod audit;
mod modsettings;
mod read;
mod safe;
mod xmlextensions;
pub use audit::{Audit, XpathSeen, audit};
pub(super) use read::wanted_by;
pub use read::{Active, Patches};

/// What one top-level operation looked at and changed. `read` includes what it wrote.
#[derive(Clone, Debug)]
pub struct Trace {
    pub mod_at: usize,
    pub file: PathBuf,
    pub read: BTreeSet<DefKey>,
    pub wrote: BTreeSet<DefKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Failed {
        mod_at: usize,
        file: PathBuf,
        operation: String,
        xpath: String,
        target: Target,
    },
    /// Two non-list elements of one name in a def, as `Duplicate XML node name` or a repeated stat.
    Duplicate { def: DefKey, element: String },
    /// A def named twice in a list of defs after inheritance.
    Twice {
        def: DefKey,
        field: String,
        entry: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Yes,
    No,
    Unsure,
}

impl From<bool> for Verdict {
    fn from(yes: bool) -> Verdict {
        if yes { Verdict::Yes } else { Verdict::No }
    }
}

/// What a failed `Sequence` reports: its last failed child as the game prints it, and where it looked.
#[derive(Default)]
struct Blame {
    text: Option<String>,
    xpath: Option<String>,
}

/// Applies each mod's patches in `order` (indexes into the mods `patches` was read from).
/// An operation whose class or XPath is not understood is `Unsure`: it never fails, and its target goes to `unknown`.
pub(super) fn apply(
    doc: &mut Document,
    patches: &Patches,
    order: &[usize],
    unknown: &mut Vec<Target>,
    traces: &mut Vec<Trace>,
    failures: &mut Vec<Failure>,
) {
    let Ok(context) = Context::new(doc.tree()) else {
        return;
    };
    for &mod_at in order {
        for file in patches.mods.get(mod_at).into_iter().flatten() {
            for operation in &file.operations {
                let mut run = Run {
                    doc: &mut *doc,
                    context: &context,
                    unknown: &mut *unknown,
                    mod_at,
                    file: &file.path,
                    read: BTreeSet::new(),
                    wrote: BTreeSet::new(),
                    touched: Vec::new(),
                    structural: false,
                };
                let mut blame = Blame::default();
                let verdict = run.apply(operation, &mut blame);
                let (read, wrote) = (run.read, run.wrote);
                traces.push(Trace {
                    mod_at,
                    file: file.path.clone(),
                    read,
                    wrote,
                });
                if verdict == Verdict::No {
                    let xpath = operation.xpath().or(blame.xpath.as_deref());
                    failures.push(Failure::Failed {
                        mod_at,
                        file: file.path.clone(),
                        operation: operation.describe(blame.text.as_deref()),
                        xpath: xpath.unwrap_or_default().to_string(),
                        target: xpath.map_or_else(Target::any, Target::of_xpath),
                    });
                }
            }
        }
    }
}

impl Operation {
    fn xpath(&self) -> Option<&str> {
        match &self.work {
            Work::Add { xpath, .. }
            | Work::Insert { xpath, .. }
            | Work::Replace { xpath, .. }
            | Work::AddModExtension { xpath, .. }
            | Work::Remove { xpath, .. }
            | Work::SetName { xpath, .. }
            | Work::Attribute { xpath, .. }
            | Work::Test { xpath }
            | Work::Conditional { xpath, .. }
            | Work::SafeRemove { xpath }
            | Work::Branch { xpath, .. }
            | Work::Slider { xpath, .. }
            | Work::Unknown { xpath, .. } => xpath.as_deref(),
            Work::Safe(safe) => safe.xpath.as_deref(),
            Work::Sequence(_)
            | Work::Nested { .. }
            | Work::Each(_)
            | Work::Fails
            | Work::FindMod { .. }
            | Work::Bare => None,
        }
    }

    /// `ToString` of the game's class: its full name, then the XPath, the attribute or the mods it was given.
    fn describe(&self, last_failed: Option<&str>) -> String {
        let class = if self.class.contains('.') {
            self.class.clone()
        } else if self.class.is_empty() {
            "Verse.PatchOperation".to_string()
        } else {
            format!("Verse.{}", self.class)
        };
        let path = self.xpath().unwrap_or_default();
        let by_framework = ["XmlExtensions.", "ModSettingsFramework."]
            .iter()
            .any(|namespace| self.class.starts_with(namespace));
        match &self.work {
            Work::Nested {
                operations,
                listed: Some(Listed { count, id }),
            } => {
                let last =
                    last_failed.map_or(String::new(), |l| format!(", lastFailedOperation={l}"));
                let _ = operations;
                format!("{class}(count={count}{last}) - {id}")
            }
            _ if by_framework => class,
            Work::Sequence(operations) => {
                let last =
                    last_failed.map_or(String::new(), |l| format!(", lastFailedOperation={l}"));
                format!("{class}(count={}{last})", operations.len())
            }
            Work::FindMod { mods, .. } => format!("{class}({})", mods.join(", ")),
            Work::Attribute { attribute, .. } => {
                format!(
                    "{class}({path})({})",
                    attribute.as_deref().unwrap_or_default()
                )
            }
            Work::Bare => class,
            _ => format!("{class}({path})"),
        }
    }
}

struct Run<'a> {
    doc: &'a mut Document,
    context: &'a Context,
    unknown: &'a mut Vec<Target>,
    mod_at: usize,
    file: &'a Path,
    read: BTreeSet<DefKey>,
    wrote: BTreeSet<DefKey>,
    touched: Vec<Node>,
    structural: bool,
}

impl Run<'_> {
    fn apply(&mut self, operation: &Operation, blame: &mut Blame) -> Verdict {
        let worked = self.work(operation, blame);
        self.flush();
        match (operation.success, worked) {
            (Success::Always, _) => Verdict::Yes,
            (Success::Never, _) => Verdict::No,
            (_, Verdict::Unsure) => Verdict::Unsure,
            (Success::Invert, worked) => Verdict::from(worked == Verdict::No),
            (Success::Normal, worked) => worked,
        }
    }

    fn work(&mut self, operation: &Operation, blame: &mut Blame) -> Verdict {
        match &operation.work {
            Work::Bare => Verdict::No,
            Work::Unknown { aimed, .. } => {
                aimed
                    .iter()
                    .for_each(|xpath| self.mark_unknown(Some(xpath)));
                Verdict::Unsure
            }
            Work::Test { xpath } => match self.nodes(xpath) {
                Ok(found) => Verdict::from(!found.is_empty()),
                Err(verdict) => verdict,
            },
            Work::Add {
                xpath,
                value,
                prepend,
                single,
            } => self.each(xpath, value, *single, |run, mut node, value| {
                if node.get_type() == Some(NodeType::ElementNode) {
                    run.mark(&node);
                    if *prepend {
                        value.iter().rev().for_each(|f| run.prepend(&mut node, f));
                    } else {
                        value.iter().for_each(|f| run.append(&mut node, f));
                    }
                }
            }),
            Work::Insert {
                xpath,
                value,
                prepend,
                single,
            } => self.each(xpath, value, *single, |run, mut node, value| {
                if run.mark_parent(&node) {
                    if *prepend {
                        value.iter().rev().for_each(|f| run.before(&mut node, f));
                    } else {
                        value.iter().for_each(|f| run.after(&mut node, f));
                    }
                }
            }),
            Work::Replace {
                xpath,
                value,
                single,
            } => self.each(xpath, value, *single, |run, mut node, value| {
                if run.mark_parent(&node) {
                    value.iter().for_each(|f| run.before(&mut node, f));
                    node.unlink_node();
                }
            }),
            Work::AddModExtension { xpath, value } => {
                self.each(xpath, value, false, |run, mut node, value| {
                    if node.get_type() != Some(NodeType::ElementNode) {
                        return;
                    }
                    run.mark(&node);
                    let list = node
                        .get_child_elements()
                        .into_iter()
                        .find(|c| c.get_name() == "modExtensions");
                    let Some(mut list) = list.or_else(|| run.created("modExtensions", &mut node))
                    else {
                        return;
                    };
                    value.iter().for_each(|f| run.append(&mut list, f));
                })
            }
            Work::Remove { xpath, single } => self.remove(xpath, *single),
            Work::SafeRemove { xpath } => match self.remove(xpath, false) {
                Verdict::No if xpath.is_some() => Verdict::Yes,
                verdict => verdict,
            },
            Work::SetName {
                xpath,
                name,
                single,
            } => {
                let Some(name) = name else {
                    return Verdict::No;
                };
                self.each(xpath, &Some(Vec::new()), *single, |run, mut node, _| {
                    if run.mark_parent(&node)
                        && let Ok(mut renamed) = Node::new(name, None, run.doc.tree())
                    {
                        for mut child in node.get_child_nodes() {
                            child.unlink_node();
                            let _ = renamed.add_child(&mut child);
                        }
                        let _ = node.add_prev_sibling(&mut renamed);
                        node.unlink_node();
                    }
                })
            }
            Work::Attribute {
                xpath,
                attribute,
                value,
                change,
            } => self.attribute(xpath, attribute.as_deref(), value, change),
            Work::Sequence(operations) | Work::Nested { operations, .. } => {
                self.sequence(operations, blame)
            }
            Work::Each(operations) => {
                for operation in operations {
                    self.apply(operation, &mut Blame::default());
                }
                Verdict::Yes
            }
            Work::Fails => Verdict::No,
            Work::Branch { xpath, yes, no } => {
                let found = match self.nodes(xpath) {
                    Ok(found) => !found.is_empty(),
                    Err(verdict) => return verdict,
                };
                self.sequence(if found { yes } else { no }, blame)
            }
            Work::Safe(safe) => self.safe(safe),
            Work::Slider {
                xpath,
                text,
                strict,
            } => self.slider(xpath, text, *strict),
            Work::Conditional {
                xpath,
                matched,
                unmatched,
            } => {
                let found = match self.nodes(xpath) {
                    Ok(found) => !found.is_empty(),
                    Err(verdict) => return verdict,
                };
                let branch = if found { matched } else { unmatched };
                if let Some(branch) = branch {
                    return self.apply(branch, &mut Blame::default());
                }
                if matched.is_none() {
                    Verdict::from(unmatched.is_some())
                } else {
                    Verdict::Yes
                }
            }
            Work::FindMod {
                found,
                matched,
                unmatched,
                ..
            } => {
                let branch = if *found { matched } else { unmatched };
                match branch {
                    Some(branch) => self.apply(branch, &mut Blame::default()),
                    None => Verdict::Yes,
                }
            }
        }
    }

    fn sequence(&mut self, operations: &[Operation], blame: &mut Blame) -> Verdict {
        let mut verdict = Verdict::Yes;
        for operation in operations {
            let mut inner = Blame::default();
            match self.apply(operation, &mut inner) {
                Verdict::No => {
                    blame.text = Some(operation.describe(inner.text.as_deref()));
                    blame.xpath = operation
                        .xpath()
                        .or(inner.xpath.as_deref())
                        .map(str::to_string);
                    return Verdict::No;
                }
                Verdict::Unsure => verdict = Verdict::Unsure,
                Verdict::Yes => {}
            }
        }
        verdict
    }

    fn attribute(
        &mut self,
        xpath: &Option<String>,
        attribute: Option<&str>,
        value: &str,
        change: &Change,
    ) -> Verdict {
        let Some(attribute) = attribute else {
            return Verdict::No;
        };
        let nodes = match self.nodes(xpath) {
            Ok(nodes) => nodes,
            Err(verdict) => return verdict,
        };
        let mut result = matches!(change, Change::Set) && !nodes.is_empty();
        for mut node in nodes {
            if node.get_type() != Some(NodeType::ElementNode) {
                continue;
            }
            let has = node.get_attribute(attribute).is_some();
            let changed = match change {
                Change::Add => !has,
                Change::Remove => has,
                Change::Set => true,
            };
            if !changed {
                continue;
            }
            result = true;
            self.mark(&node);
            let _ = match change {
                Change::Remove => node.remove_attribute(attribute),
                _ => node.set_attribute(attribute, value),
            };
        }
        Verdict::from(result)
    }

    fn remove(&mut self, xpath: &Option<String>, single: bool) -> Verdict {
        self.each(xpath, &Some(Vec::new()), single, |run, mut node, _| {
            if run.mark_parent(&node) {
                node.unlink_node();
            }
        })
    }

    fn slider(&mut self, xpath: &Option<String>, text: &str, strict: bool) -> Verdict {
        let nodes = match self.nodes(xpath) {
            Ok(nodes) => nodes,
            Err(verdict) => return verdict,
        };
        let Some(mut node) = nodes.into_iter().next() else {
            return Verdict::from(!strict);
        };
        if let Some(key) = self.doc.def_of(&node) {
            self.wrote.insert(key);
        }
        if node.get_type() == Some(NodeType::ElementNode) {
            self.mark(&node);
            let _ = node.set_content(text);
        }
        Verdict::Yes
    }

    /// Runs `change` on each match (on the first only when `single`); true when there was one. A missing
    /// `value` throws in the game before any match.
    fn each(
        &mut self,
        xpath: &Option<String>,
        value: &Option<Vec<Frag>>,
        single: bool,
        mut change: impl FnMut(&mut Self, Node, &[Frag]),
    ) -> Verdict {
        let Some(value) = value else {
            return Verdict::No;
        };
        let mut nodes = match self.nodes(xpath) {
            Ok(nodes) => nodes,
            Err(verdict) => return verdict,
        };
        if single {
            nodes.truncate(1);
        }
        let any = !nodes.is_empty();
        for node in nodes {
            if let Some(key) = self.doc.def_of(&node) {
                self.wrote.insert(key);
            }
            change(self, node, value);
        }
        Verdict::from(any)
    }

    /// The matches of `xpath`; the verdict to give instead when there is no XPath or libxml2 refuses it.
    fn nodes(&mut self, xpath: &Option<String>) -> Result<Vec<Node>, Verdict> {
        let Some(xpath) = xpath else {
            return Err(Verdict::No);
        };
        match xpath::select(self.doc, self.context, xpath) {
            Ok(selected) => {
                for node in selected.nodes.iter().chain(&selected.named) {
                    if let Some(key) = self.doc.def_of(node) {
                        self.read.insert(key);
                    }
                }
                Ok(selected.nodes)
            }
            Err(Refused) => {
                self.mark_unknown(Some(xpath));
                Err(Verdict::Unsure)
            }
        }
    }

    fn mark_unknown(&mut self, xpath: Option<&str>) {
        self.unknown
            .push(xpath.map_or_else(Target::any, Target::of_xpath));
    }

    /// `node`'s own content or attributes change.
    fn mark(&mut self, node: &Node) {
        if node.to_hashable() == self.doc.root().to_hashable() {
            self.structural = true;
        } else {
            self.touched.push(node.clone());
        }
    }

    /// `node` is replaced, removed or has siblings added: its parent changes. False when it has no element parent.
    fn mark_parent(&mut self, node: &Node) -> bool {
        let parent = node
            .get_parent()
            .filter(|p| p.get_type() == Some(NodeType::ElementNode));
        let Some(parent) = parent else {
            return false;
        };
        self.mark(&parent);
        true
    }

    fn flush(&mut self) {
        for node in std::mem::take(&mut self.touched) {
            self.doc.rekey(&node);
        }
        if std::mem::take(&mut self.structural) {
            let added = self.doc.resync(self.mod_at, self.file);
            self.wrote.extend(added);
        }
    }

    fn created(&self, name: &str, parent: &mut Node) -> Option<Node> {
        let mut node = Node::new(name, None, self.doc.tree()).ok()?;
        parent.add_child(&mut node).ok()?;
        Some(node)
    }

    fn build(&self, frag: &Frag) -> Option<Node> {
        let tree = self.doc.tree();
        let (name, attributes, children) = match frag {
            Frag::Text(text) => return Node::new_text(text, tree).ok(),
            Frag::Element {
                name,
                attributes,
                children,
            } => (name, attributes, children),
        };
        let mut node = Node::new(name, None, tree).ok()?;
        for (key, value) in attributes {
            node.set_attribute(key, value).ok()?;
        }
        for child in children {
            self.append(&mut node, child);
        }
        Some(node)
    }

    // libxml2 merges a text node into a text neighbour and frees it, so text is merged by hand.
    fn append(&self, parent: &mut Node, frag: &Frag) {
        match frag {
            Frag::Text(text) => {
                let _ = parent.append_text(text);
            }
            _ => {
                if let Some(mut node) = self.build(frag) {
                    let _ = parent.add_child(&mut node);
                }
            }
        }
    }

    fn prepend(&self, parent: &mut Node, frag: &Frag) {
        match parent.get_first_child() {
            Some(mut first) => self.before(&mut first, frag),
            None => self.append(parent, frag),
        }
    }

    fn before(&self, anchor: &mut Node, frag: &Frag) {
        if let Frag::Text(text) = frag {
            if anchor.is_text_node() {
                let _ = anchor.set_content(&format!("{text}{}", anchor.get_content()));
                return;
            }
            if let Some(mut previous) = anchor.get_prev_sibling().filter(Node::is_text_node) {
                let _ = previous.append_text(text);
                return;
            }
        }
        if let Some(mut node) = self.build(frag) {
            let _ = anchor.add_prev_sibling(&mut node);
        }
    }

    fn after(&self, anchor: &mut Node, frag: &Frag) {
        if let Frag::Text(text) = frag {
            if anchor.is_text_node() {
                let _ = anchor.append_text(text);
                return;
            }
            if let Some(mut next) = anchor.get_next_sibling().filter(Node::is_text_node) {
                let _ = next.set_content(&format!("{text}{}", next.get_content()));
                return;
            }
        }
        if let Some(mut node) = self.build(frag) {
            let _ = anchor.add_next_sibling(&mut node);
        }
    }
}

#[cfg(test)]
mod tests;
