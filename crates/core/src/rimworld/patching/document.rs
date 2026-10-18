use super::{DefKey, ModFiles};
use crate::note;
use libxml::parser::Parser;
use libxml::readonly::RoNode;
use libxml::tree::{Document as Tree, Node};
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Default)]
pub(super) struct Origin {
    pub place: usize,
    pub file: PathBuf,
    /// Put in by a patch: the game has no mod for such a node.
    pub added: bool,
}

struct Def {
    node: Node,
    key: Option<DefKey>,
    attribute: Option<DefKey>,
    origin: Origin,
}

pub(super) struct Document {
    tree: Tree,
    defs: Vec<Def>,
    index: HashMap<DefKey, Vec<usize>>,
    by_attribute: HashMap<DefKey, Vec<usize>>,
    by_node: HashMap<usize, usize>,
    /// Whatever the kind: a `ParentName` does not say it. Built when first asked for, dropped by `reindex`.
    by_name: OnceLock<HashMap<String, Vec<usize>>>,
    /// The defs by kind and `ParentName`, built and dropped like `by_name`.
    by_parent: OnceLock<HashMap<DefKey, Vec<usize>>>,
}

pub(super) struct Top {
    pub node: RoNode,
    pub key: Option<DefKey>,
    pub place: Option<usize>,
}

struct Part {
    xml: String,
    origin: Origin,
}

impl Document {
    pub(super) fn load(mods: &[ModFiles]) -> Document {
        let files: Vec<(usize, PathBuf)> = mods
            .iter()
            .enumerate()
            .flat_map(|(place, files)| {
                files_in_game_order(files, "Defs")
                    .into_iter()
                    .map(move |file| (place, file))
            })
            .collect();
        let parts = files
            .par_iter()
            .flat_map_iter(|(place, file)| parts_of(*place, file))
            .collect();
        Document::assemble(parts)
    }

    pub(super) fn len(&self) -> usize {
        self.defs.len()
    }

    pub(super) fn tree(&self) -> &Tree {
        &self.tree
    }

    #[cfg(test)]
    pub(super) fn nodes_of(&self, key: &DefKey) -> Vec<Node> {
        let found = self.index.get(key).into_iter().flatten();
        found.map(|&at| self.defs[at].node.clone()).collect()
    }

    pub(super) fn def_of(&self, node: &Node) -> Option<DefKey> {
        self.def_index_of(node)
            .and_then(|at| self.defs[at].key.clone())
    }

    #[cfg(test)]
    pub(super) fn origin_of(&self, node: &Node) -> Option<&Origin> {
        self.def_index_of(node).map(|at| &self.defs[at].origin)
    }

    /// The defs in `keep` and every def a `ParentName` leads to, in document order.
    pub(super) fn subset(&self, keep: &BTreeSet<DefKey>) -> Document {
        let mut chosen: HashSet<usize> = (0..self.defs.len())
            .filter(|&at| self.defs[at].key.as_ref().is_some_and(|k| keep.contains(k)))
            .collect();
        let mut queue: Vec<usize> = chosen.iter().copied().collect();
        let by_name = self.by_name.get_or_init(|| {
            let mut names: HashMap<String, Vec<usize>> = HashMap::new();
            for (at, def) in self.defs.iter().enumerate() {
                if let Some(key) = &def.attribute {
                    names.entry(key.name.clone()).or_default().push(at);
                }
            }
            names
        });
        while let Some(at) = queue.pop() {
            let Some(parent) = self.defs[at].node.get_attribute("ParentName") else {
                continue;
            };
            for &other in by_name.get(&parent).into_iter().flatten() {
                if chosen.insert(other) {
                    queue.push(other);
                }
            }
        }
        let parts = (0..self.defs.len())
            .filter(|at| chosen.contains(at))
            .map(|at| {
                let def = &self.defs[at];
                Part {
                    xml: self.tree.node_to_string(&def.node),
                    origin: Origin {
                        place: def.origin.place,
                        file: def.origin.file.clone(),
                        added: def.origin.added,
                    },
                }
            })
            .collect();
        Document::assemble(parts)
    }

    pub(super) fn copy(&self) -> Document {
        let tree = self.tree.dup().expect("libxml2 copies a document");
        let root = tree.get_root_element().expect("the copy has <Defs>");
        let nodes = root.get_child_elements();
        debug_assert_eq!(nodes.len(), self.defs.len());
        let mut copy = Document {
            tree,
            defs: Vec::new(),
            index: HashMap::new(),
            by_attribute: HashMap::new(),
            by_node: HashMap::new(),
            by_name: OnceLock::new(),
            by_parent: OnceLock::new(),
        };
        for (node, def) in nodes.into_iter().zip(&self.defs) {
            copy.defs.push(Def {
                node,
                key: def.key.clone(),
                attribute: def.attribute.clone(),
                origin: Origin {
                    place: def.origin.place,
                    file: def.origin.file.clone(),
                    added: def.origin.added,
                },
            });
        }
        copy.reindex();
        copy
    }

    /// Puts the top-level nodes in load order, `rank` giving each place its position, and drops those of
    /// a place that has none.
    pub(super) fn arrange(&mut self, rank: &[Option<usize>]) {
        let rank_of = |def: &Def| rank.get(def.origin.place).copied().flatten();
        let mut defs = std::mem::take(&mut self.defs);
        defs.retain_mut(|def| {
            let kept = rank_of(def).is_some();
            if !kept {
                def.node.unlink_node();
            }
            kept
        });
        if !defs.is_sorted_by_key(&rank_of) {
            defs.sort_by_key(|def| rank_of(def));
            let mut root = self.root();
            for def in &mut defs {
                def.node.unlink_node();
                let _ = root.add_child(&mut def.node);
            }
        }
        self.defs = defs;
        self.reindex();
    }

    /// The top-level nodes in document order, each with its key and the place it was read from (none for
    /// a node a patch added).
    pub(super) fn tops(&self) -> Vec<Top> {
        let nodes = self
            .tree
            .get_root_readonly()
            .map(RoNode::get_child_elements);
        let nodes = nodes.unwrap_or_default();
        debug_assert!(
            nodes
                .iter()
                .zip(&self.defs)
                .all(|(ro, def)| ro.to_hashable() == def.node.to_hashable())
        );
        self.defs
            .iter()
            .zip(nodes)
            .map(|(def, node)| Top {
                node,
                key: def.key.clone(),
                place: (!def.origin.added).then_some(def.origin.place),
            })
            .collect()
    }

    fn def_index_of(&self, node: &Node) -> Option<usize> {
        let mut at = node.clone();
        loop {
            if let Some(&found) = self.by_node.get(&at.to_hashable()) {
                return Some(found);
            }
            at = at.get_parent()?;
        }
    }

    fn assemble(mut parts: Vec<Part>) -> Document {
        let tree = match combined(&parts) {
            Some(tree) => tree,
            None => {
                parts.retain(|part| {
                    let fine = combined(std::slice::from_ref(part)).is_some();
                    if !fine {
                        note!(
                            "Patch simulation skips a def of {}: it does not parse on its own",
                            part.origin.file.display()
                        );
                    }
                    fine
                });
                combined(&parts).unwrap_or_else(|| combined(&[]).expect("an empty <Defs> parses"))
            }
        };
        let root = tree.get_root_element().expect("the document has <Defs>");
        let nodes = root.get_child_elements();
        debug_assert_eq!(nodes.len(), parts.len());
        let mut document = Document {
            tree,
            defs: Vec::new(),
            index: HashMap::new(),
            by_attribute: HashMap::new(),
            by_node: HashMap::new(),
            by_name: OnceLock::new(),
            by_parent: OnceLock::new(),
        };
        for (node, part) in nodes.into_iter().zip(parts) {
            document.defs.push(Def::new(node, part.origin));
        }
        document.reindex();
        document
    }

    fn reindex(&mut self) {
        self.index.clear();
        self.by_attribute.clear();
        self.by_node.clear();
        self.by_name = OnceLock::new();
        self.by_parent = OnceLock::new();
        for (at, def) in self.defs.iter().enumerate() {
            if let Some(key) = &def.key {
                self.index.entry(key.clone()).or_default().push(at);
            }
            if let Some(key) = &def.attribute {
                self.by_attribute.entry(key.clone()).or_default().push(at);
            }
            self.by_node.insert(def.node.to_hashable(), at);
        }
    }

    pub(super) fn root(&self) -> Node {
        self.tree
            .get_root_element()
            .expect("the document has <Defs>")
    }

    /// The top-level `kind` nodes whose `defName` (or, with `true`, `Name` attribute) is one of `names`,
    /// in document order. The index keys on trimmed text, so the XPath still has to confirm each.
    pub(super) fn candidates(&self, kind: &str, names: &[(bool, &str)]) -> Vec<Node> {
        let mut found: Vec<usize> = Vec::new();
        for &(by_attribute, name) in names {
            let key = DefKey {
                kind: kind.to_string(),
                name: name.trim().to_string(),
            };
            let table = if by_attribute {
                &self.by_attribute
            } else {
                &self.index
            };
            found.extend(table.get(&key).into_iter().flatten());
        }
        found.sort_unstable();
        found.dedup();
        found
            .into_iter()
            .map(|at| self.defs[at].node.clone())
            .collect()
    }

    /// The keys of the top-level `kind` defs whose `Name` attribute is exactly `name`.
    pub(super) fn keys_named(&self, kind: &str, name: &str) -> Vec<DefKey> {
        let key = DefKey {
            kind: kind.to_string(),
            name: name.to_string(),
        };
        let found = self.by_attribute.get(&key).into_iter().flatten();
        found.filter_map(|&at| self.defs[at].key.clone()).collect()
    }

    /// The keys of the top-level `kind` defs whose `ParentName` attribute is exactly `parent`.
    pub(super) fn keys_derived(&self, kind: &str, parent: &str) -> Vec<DefKey> {
        let by_parent = self.by_parent.get_or_init(|| {
            let mut found: HashMap<DefKey, Vec<usize>> = HashMap::new();
            for (at, def) in self.defs.iter().enumerate() {
                if let Some(name) = def.node.get_attribute("ParentName") {
                    let kind = def.node.get_name();
                    found.entry(DefKey { kind, name }).or_default().push(at);
                }
            }
            found
        });
        let key = DefKey {
            kind: kind.to_string(),
            name: parent.to_string(),
        };
        let found = by_parent.get(&key).into_iter().flatten();
        found.filter_map(|&at| self.defs[at].key.clone()).collect()
    }

    /// Re-reads the keys of the def holding `node` after an operation changed something inside it.
    pub(super) fn rekey(&mut self, node: &Node) {
        let Some(at) = self.def_index_of(node) else {
            return;
        };
        let fresh = Def::new(self.defs[at].node.clone(), Origin::default());
        let def = &mut self.defs[at];
        if fresh.key != def.key || fresh.attribute != def.attribute {
            def.key = fresh.key;
            def.attribute = fresh.attribute;
            self.reindex();
        }
    }

    /// After top-level nodes were added or removed: rebuilds the list from `<Defs>`'s children, giving a
    /// node that was not there before this place and file. Returns the keys of the new ones.
    pub(super) fn resync(&mut self, place: usize, file: &Path) -> Vec<DefKey> {
        let mut old: HashMap<usize, Def> = std::mem::take(&mut self.defs)
            .into_iter()
            .map(|def| (def.node.to_hashable(), def))
            .collect();
        let mut added = Vec::new();
        for node in self.root().get_child_elements() {
            let def = old.remove(&node.to_hashable()).unwrap_or_else(|| {
                let origin = Origin {
                    place,
                    file: file.to_path_buf(),
                    added: true,
                };
                let def = Def::new(node, origin);
                added.extend(def.key.clone());
                def
            });
            self.defs.push(def);
        }
        self.reindex();
        added
    }
}

fn combined(parts: &[Part]) -> Option<Tree> {
    let xml: String = parts.iter().map(|part| part.xml.as_str()).collect();
    Parser::default()
        .parse_string(format!("<Defs>{xml}</Defs>"))
        .ok()
}

impl Def {
    fn new(node: Node, origin: Origin) -> Def {
        let attribute = node.get_attribute("Name").map(|name| DefKey {
            kind: node.get_name(),
            name,
        });
        Def {
            key: key_of(&node),
            attribute,
            node,
            origin,
        }
    }
}

fn key_of(node: &Node) -> Option<DefKey> {
    let named = node
        .get_child_elements()
        .into_iter()
        .find(|child| child.get_name() == "defName")
        .map(|child| child.get_content().trim().to_string())
        .filter(|name| !name.is_empty());
    let name = named.or_else(|| node.get_attribute("Name"))?;
    Some(DefKey {
        kind: node.get_name(),
        name,
    })
}

/// A file whose path under `below` (`Defs` or `Patches`) was already seen in a more specific folder is skipped.
pub(super) fn files_in_game_order(files: &ModFiles, below: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for folder in files.folders.iter().rev() {
        let base = folder.join(below);
        let mut here = Vec::new();
        collect(&base, &mut here);
        here.sort_by_cached_key(|path| relative(&base, path));
        for path in here {
            if seen.insert(relative(&base, &path)) {
                found.push(path);
            }
        }
    }
    found
}

fn relative(base: &Path, path: &Path) -> String {
    let inside = path.strip_prefix(base).unwrap_or(path);
    inside.to_string_lossy().replace('\\', "/")
}

fn collect(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, into);
        } else if !entry.file_name().to_string_lossy().starts_with('.')
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
        {
            into.push(path);
        }
    }
}

fn parts_of(place: usize, file: &Path) -> Vec<Part> {
    let parsed = std::fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|text| {
            Parser::default()
                .parse_string(text)
                .map_err(|e| e.to_string())
        });
    let tree = match parsed {
        Ok(tree) => tree,
        Err(why) => {
            note!("Patch simulation skips {}: {why}", file.display());
            return Vec::new();
        }
    };
    let Some(root) = tree.get_root_element().filter(|r| r.get_name() == "Defs") else {
        note!("Patch simulation skips {}: no <Defs> root", file.display());
        return Vec::new();
    };
    root.get_child_elements()
        .iter()
        .map(|node| Part {
            xml: tree.node_to_string(node),
            origin: Origin {
                place,
                file: file.to_path_buf(),
                added: false,
            },
        })
        .collect()
}
