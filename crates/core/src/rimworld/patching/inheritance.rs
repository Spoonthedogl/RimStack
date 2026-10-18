use super::operations::wanted_by;
use super::{Active, DefKey};
use libxml::readonly::RoNode;
use libxml::tree::NodeType;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone)]
pub(super) enum Child {
    Text(String),
    Element(Elem),
}

/// An element as the game's `XmlInheritance` sees it: whitespace between elements is not a node.
#[derive(Clone)]
pub(super) struct Elem {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Child>,
}

impl Elem {
    /// A value `li` whose `MayRequire` names a mod that is not active is left out, as the game does.
    pub(super) fn of(node: RoNode, active: &Active) -> Elem {
        let mut children = Vec::new();
        let mut next = node.get_first_child();
        while let Some(child) = next {
            next = child.get_next_sibling();
            match child.get_type() {
                Some(NodeType::ElementNode) => {
                    let elem = Elem::of(child, active);
                    if elem.name != "li" || wanted_by(&elem.attrs, active) {
                        children.push(Child::Element(elem));
                    }
                }
                Some(NodeType::TextNode | NodeType::CDataSectionNode) => {
                    let text = child.get_content();
                    if !text.trim().is_empty() {
                        children.push(Child::Text(text));
                    }
                }
                _ => {}
            }
        }
        Elem {
            name: node.get_name(),
            attrs: node.get_attributes().into_iter().collect(),
            children,
        }
    }

    pub(super) fn elements(&self) -> impl Iterator<Item = &Elem> {
        self.children.iter().filter_map(|child| match child {
            Child::Element(e) => Some(e),
            Child::Text(_) => None,
        })
    }

    pub(super) fn text(&self) -> Option<&str> {
        self.children.iter().rev().find_map(|child| match child {
            Child::Text(t) => Some(t.as_str()),
            Child::Element(_) => None,
        })
    }
}

pub(super) struct Source {
    pub key: Option<DefKey>,
    /// Position of the node's mod in the load order; none for a node no mod owns.
    pub rank: Option<usize>,
    pub elem: Elem,
}

/// A source after inheritance.
pub(super) enum Slot {
    /// Nothing was inherited: the source's own element.
    Own,
    Merged(Elem),
    /// Its hierarchy is cyclic.
    Cyclic,
}

impl Slot {
    pub(super) fn get<'a>(&'a self, own: &'a Elem) -> Option<&'a Elem> {
        match self {
            Slot::Own => Some(own),
            Slot::Merged(elem) => Some(elem),
            Slot::Cyclic => None,
        }
    }
}

/// Each source after inheritance, and the elements named twice in the own XML of a node that has a
/// parent (`CheckForDuplicateNodes`).
pub(super) fn resolve(sources: &[Source]) -> (Vec<Slot>, Vec<(DefKey, String)>) {
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut registered = Vec::new();
    for (at, source) in sources.iter().enumerate() {
        let attrs = &source.elem.attrs;
        if !attrs.contains_key("Name") && !attrs.contains_key("ParentName") {
            continue;
        }
        if let Some(name) = attrs.get("Name") {
            let same = by_name.get(name.as_str());
            if same.is_some_and(|l| l.iter().any(|&o| sources[o].rank == source.rank)) {
                continue;
            }
            by_name.entry(name).or_default().push(at);
        }
        registered.push(at);
    }
    let mut parents: HashMap<usize, usize> = HashMap::new();
    let mut duplicates = Vec::new();
    for &at in &registered {
        let Some(parent_name) = sources[at].elem.attrs.get("ParentName") else {
            continue;
        };
        let candidates = by_name.get(parent_name.as_str());
        if let Some(parent) = candidates.and_then(|c| best_parent(sources, c, sources[at].rank)) {
            parents.insert(at, parent);
            if let Some(key) = &sources[at].key {
                let mut found = Vec::new();
                repeated(&sources[at].elem, &mut found);
                duplicates.extend(found.into_iter().map(|name| (key.clone(), name)));
            }
        }
    }
    let mut resolved: Vec<Slot> = sources.iter().map(|_| Slot::Own).collect();
    let mut state = vec![0u8; sources.len()];
    for &at in &registered {
        settle(at, sources, &parents, &mut resolved, &mut state);
    }
    (resolved, duplicates)
}

/// The latest node of the name at or before the child's mod, else one no mod owns; for a child no mod
/// owns, the first no mod owns, else the earliest mod's.
fn best_parent(sources: &[Source], named: &[usize], mine: Option<usize>) -> Option<usize> {
    let rank = |at: usize| sources[at].rank;
    let ownerless = || named.iter().copied().find(|&at| rank(at).is_none());
    match mine {
        None => ownerless().or_else(|| {
            let mut best: Option<usize> = None;
            for &at in named {
                if best.is_none_or(|b| rank(at) < rank(b)) {
                    best = Some(at);
                }
            }
            best
        }),
        Some(mine) => {
            let mut best: Option<usize> = None;
            for &at in named {
                if rank(at).is_some_and(|r| r <= mine) && best.is_none_or(|b| rank(at) > rank(b)) {
                    best = Some(at);
                }
            }
            best.or_else(ownerless)
        }
    }
}

fn settle(
    at: usize,
    sources: &[Source],
    parents: &HashMap<usize, usize>,
    resolved: &mut Vec<Slot>,
    state: &mut Vec<u8>,
) -> bool {
    let sound = |resolved: &[Slot], at: usize| !matches!(resolved[at], Slot::Cyclic);
    match state[at] {
        2 => return sound(resolved, at),
        1 => {
            resolved[at] = Slot::Cyclic;
            return false;
        }
        _ => {}
    }
    state[at] = 1;
    if let Some(&parent) = parents.get(&at) {
        if settle(parent, sources, parents, resolved, state) && sound(resolved, at) {
            let mut merged = resolved[parent]
                .get(&sources[parent].elem)
                .expect("a settled parent")
                .clone();
            overwrite(&sources[at].elem, &mut merged);
            resolved[at] = Slot::Merged(merged);
        } else {
            resolved[at] = Slot::Cyclic;
        }
    }
    state[at] = 2;
    sound(resolved, at)
}

fn is_list(elem: &Elem) -> bool {
    elem.name == "li"
}

fn repeated(elem: &Elem, found: &mut Vec<String>) {
    let mut used = HashSet::new();
    for child in elem.elements() {
        if !is_list(child) && !used.insert(child.name.as_str()) {
            found.push(child.name.clone());
        }
        repeated(child, found);
    }
}

/// `RecursiveNodeCopyOverwriteElements`.
fn overwrite(child: &Elem, current: &mut Elem) {
    if child
        .attrs
        .get("Inherit")
        .is_some_and(|v| v.eq_ignore_ascii_case("false"))
    {
        current.children = child.children.clone();
        for (name, value) in &child.attrs {
            if name != "Inherit" {
                current.attrs.insert(name.clone(), value.clone());
            }
        }
        return;
    }
    current.attrs = child.attrs.clone();
    if let Some(text) = child.text() {
        current.children = vec![Child::Text(text.to_string())];
        return;
    }
    if child.elements().next().is_none() {
        if current.elements().next().is_none() {
            current.children.clear();
        }
        return;
    }
    for item in child.elements() {
        if is_list(item) {
            current.children.push(Child::Element(item.clone()));
            continue;
        }
        let same = current.children.iter_mut().find_map(|c| match c {
            Child::Element(e) if e.name == item.name => Some(e),
            _ => None,
        });
        match same {
            Some(existing) => overwrite(item, existing),
            None => current.children.push(Child::Element(item.clone())),
        }
    }
}
