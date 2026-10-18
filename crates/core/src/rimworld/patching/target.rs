use super::DefKey;
use super::xpath;
use std::collections::BTreeSet;

/// What an XPath is known to reach among the top-level defs: the union of its sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    sides: Vec<Side>,
}

/// One side of a union; a `None` is any.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Side {
    kind: Option<String>,
    names: Option<BTreeSet<String>>,
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
            [side] => side.names.as_ref(),
            _ => None,
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

    fn covers(&self, def: &DefKey) -> bool {
        self.kind.as_ref().is_none_or(|kind| *kind == def.kind)
            && self
                .names
                .as_ref()
                .is_none_or(|names| names.contains(&def.name))
    }

    fn overlaps(&self, other: &Side) -> bool {
        let kinds = self.kind.is_none() || other.kind.is_none() || self.kind == other.kind;
        let names = match (&self.names, &other.names) {
            (Some(a), Some(b)) => !a.is_disjoint(b),
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

/// A `@Name` test widens to every name: a def with a `defName` is keyed by that, which its `Name` does not tell.
fn name_predicate(rest: &str) -> Option<BTreeSet<String>> {
    let tests = xpath::name_tests(rest)?.tests;
    if tests.iter().any(|(by_attribute, _)| *by_attribute) {
        return None;
    }
    Some(tests.into_iter().map(|(_, name)| name).collect())
}

#[cfg(test)]
mod tests;
