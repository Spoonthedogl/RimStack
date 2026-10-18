//! XmlExtensions' `PatchOperationSafe` family: each child of `value` is looked for among the children
//! of the matched node before it is added or replaces the one found.

use super::read::Frag;
use super::{Run, Verdict};
use libxml::tree::{Node, NodeType};

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Add,
    Replace,
    AddOrReplace,
    /// `PatchOperationAddOrReplace`: a child found is replaced whole, never entered.
    Flat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Compare {
    Name,
    InnerText,
    Both,
}

pub(super) struct Safe {
    pub kind: Kind,
    pub xpath: Option<String>,
    pub single: bool,
    pub value: Option<Vec<Frag>>,
    /// `safetyDepth`; below 0 the game takes 99999.
    pub depth: usize,
    pub attributes: bool,
    pub compare: Compare,
}

impl Frag {
    fn name(&self) -> &str {
        match self {
            Frag::Element { name, .. } => name,
            Frag::Text(_) => "#text",
        }
    }

    fn inner_text(&self) -> String {
        match self {
            Frag::Element { children, .. } => children.iter().map(Frag::inner_text).collect(),
            Frag::Text(text) => text.clone(),
        }
    }

    /// `HasChildNodes && FirstChild.HasChildNodes`: a leaf like `<a>text</a>` is not entered.
    fn entered(&self) -> bool {
        match self {
            Frag::Element { children, .. } => matches!(
                children.first(),
                Some(Frag::Element { children, .. }) if !children.is_empty()
            ),
            Frag::Text(_) => false,
        }
    }

    fn children(&self) -> &[Frag] {
        match self {
            Frag::Element { children, .. } => children,
            Frag::Text(_) => &[],
        }
    }
}

fn name_of(node: &Node) -> String {
    match node.get_type() {
        Some(NodeType::ElementNode) => node.get_name(),
        Some(NodeType::TextNode | NodeType::CDataSectionNode) => "#text".to_string(),
        _ => "#other".to_string(),
    }
}

impl Run<'_> {
    pub(super) fn safe(&mut self, safe: &Safe) -> Verdict {
        let Some(value) = &safe.value else {
            return Verdict::No;
        };
        let mut nodes = match self.nodes(&safe.xpath) {
            Ok(nodes) => nodes,
            Err(verdict) => return verdict,
        };
        if nodes.is_empty() {
            return Verdict::No;
        }
        if safe.single {
            nodes.truncate(1);
        }
        for mut node in nodes {
            if let Some(key) = self.doc.def_of(&node) {
                self.wrote.insert(key);
            }
            if node.get_type() != Some(NodeType::ElementNode) {
                continue;
            }
            for frag in value {
                match safe.kind {
                    Kind::Flat => match self.contains(&node, frag, safe) {
                        Some(found) => self.replace(&mut node, found, frag),
                        None => self.add(&mut node, frag),
                    },
                    _ => self.merge(&mut node, frag, 0, safe),
                }
            }
        }
        Verdict::Yes
    }

    fn merge(&mut self, parent: &mut Node, frag: &Frag, depth: usize, safe: &Safe) {
        let Some(mut found) = self.contains(parent, frag, safe) else {
            if !matches!(safe.kind, Kind::Replace) {
                self.add(parent, frag);
            }
            return;
        };
        let deep = depth >= safe.depth;
        let enter = !deep && frag.entered();
        match safe.kind {
            Kind::Add if deep => self.add(parent, frag),
            Kind::Add if enter => self.merge_into(&mut found, frag, depth, safe),
            Kind::Replace | Kind::AddOrReplace if enter => {
                self.merge_into(&mut found, frag, depth, safe)
            }
            Kind::Replace | Kind::AddOrReplace => self.replace(parent, found, frag),
            Kind::Add | Kind::Flat => {}
        }
    }

    fn merge_into(&mut self, found: &mut Node, frag: &Frag, depth: usize, safe: &Safe) {
        for inner in frag.children() {
            self.merge(found, inner, depth + 1, safe);
        }
    }

    fn add(&mut self, parent: &mut Node, frag: &Frag) {
        self.mark(parent);
        self.append(parent, frag);
    }

    fn replace(&mut self, parent: &mut Node, mut found: Node, frag: &Frag) {
        self.mark(parent);
        if let (Frag::Text(text), true) = (frag, found.is_text_node()) {
            let _ = found.set_content(text);
            return;
        }
        self.after(&mut found, frag);
        found.unlink_node();
    }

    /// The first child of `parent` that `frag` stands for, by `PatchOperationSafe.ContainsNode`.
    fn contains(&self, parent: &Node, frag: &Frag, safe: &Safe) -> Option<Node> {
        let text = frag.inner_text();
        let entered = frag.entered();
        let compare = safe.compare;
        parent.get_child_nodes().into_iter().find(|child| {
            let same_name = name_of(child) == frag.name();
            let same_text = child.get_content() == text;
            let candidate = (same_name && (compare == Compare::Name || entered))
                || (same_text && compare == Compare::InnerText)
                || (same_text && same_name && compare == Compare::Both);
            candidate && (!safe.attributes || same_attributes(child, frag))
        })
    }
}

fn same_attributes(child: &Node, frag: &Frag) -> bool {
    let wanted = match frag {
        Frag::Element { attributes, .. } => Some(attributes),
        Frag::Text(_) => None,
    };
    let is_element = child.get_type() == Some(NodeType::ElementNode);
    match (wanted, is_element) {
        (None, false) => true,
        (Some(wanted), true) => {
            let have = child.get_attributes();
            have.len() == wanted.len()
                && wanted
                    .iter()
                    .all(|(k, v)| have.get(k).is_some_and(|h| h == v))
        }
        _ => false,
    }
}
