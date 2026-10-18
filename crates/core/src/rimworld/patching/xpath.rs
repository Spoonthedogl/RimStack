use super::document::Document;
use libxml::tree::Node;
use libxml::xpath::Context;

/// libxml2 would not evaluate the expression.
pub(super) struct Refused;

/// The matches, and the top-level defs a first step named whether or not the rest matched anything.
pub(super) struct Selected {
    pub nodes: Vec<Node>,
    pub named: Vec<Node>,
}

/// Evaluated as the game does, from the document. A first step naming a top-level def by `defName` or
/// `@Name` is found through the document's index and only the rest goes through libxml2.
pub(super) fn select(doc: &Document, context: &Context, xpath: &str) -> Result<Selected, Refused> {
    if let Some(step) = indexed(xpath) {
        return through_index(doc, context, &step);
    }
    let from = doc.root().get_parent().ok_or(Refused)?;
    Ok(Selected {
        nodes: evaluate(context, xpath, &from)?,
        named: Vec::new(),
    })
}

fn evaluate(context: &Context, xpath: &str, from: &Node) -> Result<Vec<Node>, Refused> {
    context
        .node_evaluate_checked(xpath, from)
        .map(|found| found.get_nodes_as_vec())
        .map_err(|_| Refused)
}

fn through_index(doc: &Document, context: &Context, step: &Indexed) -> Result<Selected, Refused> {
    let names: Vec<(bool, &str)> = step.names.iter().map(|(a, n)| (*a, n.as_str())).collect();
    let expression = format!("self::{}[{}]{}", step.kind, step.predicate, step.rest);
    let named = doc.candidates(step.kind, &names);
    let mut nodes = Vec::new();
    for def in &named {
        nodes.extend(evaluate(context, &expression, def)?);
    }
    Ok(Selected { nodes, named })
}

struct Indexed<'a> {
    kind: &'a str,
    predicate: &'a str,
    rest: &'a str,
    names: Vec<(bool, String)>,
}

/// What follows the root step. Evaluated from the document node, a leading `*` can only be `<Defs>`.
pub(super) fn below_defs(path: &str) -> Option<&str> {
    path.strip_prefix("Defs/")
        .or_else(|| path.strip_prefix("*/"))
}

/// `Defs/Kind[defName="a" or @Name='b']/rest`, and nothing else.
fn indexed(xpath: &str) -> Option<Indexed<'_>> {
    let path = xpath.trim();
    let below = below_defs(path.strip_prefix('/').unwrap_or(path))?;
    let open = below.find('[')?;
    let kind = &below[..open];
    let plain = |c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '-');
    if kind.is_empty() || !kind.chars().all(plain) {
        return None;
    }
    let NameTests {
        predicate,
        rest,
        tests: names,
    } = name_tests(&below[open..])?;
    let names = names
        .into_iter()
        .map(Test::into_pair)
        .collect::<Option<Vec<_>>>()?;
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    Some(Indexed {
        kind,
        predicate,
        rest,
        names,
    })
}

/// A predicate that narrows to names.
pub(super) struct NameTests<'a> {
    pub predicate: &'a str,
    pub rest: &'a str,
    pub tests: Vec<Test>,
}

/// A condition that holds for the defs with one name: as their `defName`, as their `Name` attribute, or as
/// the `ParentName` attribute they inherit by.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Test {
    DefName(String),
    Name(String),
    ParentName(String),
}

impl Test {
    /// Whether it is the `Name` attribute, and the name; the document's index has no `ParentName`.
    fn into_pair(self) -> Option<(bool, String)> {
        match self {
            Test::DefName(name) => Some((false, name)),
            Test::Name(name) => Some((true, name)),
            Test::ParentName(_) => None,
        }
    }
}

type Tests = Vec<Test>;

/// From a `[`: the predicate, what follows its `]`, and the names it narrows to, when it does.
pub(super) fn name_tests(text: &str) -> Option<NameTests<'_>> {
    let inside = text.strip_prefix('[')?;
    let close = closing_bracket(inside)?;
    let (predicate, rest) = (&inside[..close], &inside[close + 1..]);
    Some(NameTests {
        predicate,
        rest,
        tests: names_of(predicate)?,
    })
}

/// The names a condition is true for at most, or `None` when it does not narrow. An `or` narrows only
/// when every side does; an `and` narrows to what any of its sides does, and to their common names when
/// several do.
fn names_of(condition: &str) -> Option<Tests> {
    let condition = condition.trim();
    let alternatives = split_on(condition, "or");
    if alternatives.len() > 1 {
        let mut all = Tests::new();
        for alternative in alternatives {
            all.extend(names_of(alternative)?);
        }
        return Some(all);
    }
    let conjuncts = split_on(condition, "and");
    if conjuncts.len() > 1 {
        return common(conjuncts.into_iter().filter_map(names_of).collect());
    }
    if let Some(inside) = parenthesized(condition) {
        return names_of(inside);
    }
    name_test(condition).map(|test| vec![test])
}

/// Names joined by `and`. An attribute among them is not told apart from a `defName` without the document, so all are kept.
fn common(mut sides: Vec<Tests>) -> Option<Tests> {
    if sides.len() < 2 {
        return sides.pop();
    }
    let mut first = sides.remove(0);
    if sides
        .iter()
        .chain([&first])
        .flatten()
        .any(|test| !matches!(test, Test::DefName(_)))
    {
        for side in sides {
            first.extend(side);
        }
        first.sort();
        first.dedup();
        return Some(first);
    }
    first.retain(|test| sides.iter().all(|side| side.contains(test)));
    Some(first)
}

fn parenthesized(condition: &str) -> Option<&str> {
    let inside = condition.strip_prefix('(')?;
    let mut depth = 0usize;
    let mut quote = None;
    for (at, c) in inside.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '(') => depth += 1,
            (None, ')') if depth == 0 => return (at + 1 == inside.len()).then(|| &inside[..at]),
            (None, ')') => depth -= 1,
            _ => {}
        }
    }
    None
}

fn closing_bracket(text: &str) -> Option<usize> {
    let (mut quote, mut depth) = (None, 0usize);
    for (at, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '[') => depth += 1,
            (None, ']') if depth == 0 => return Some(at),
            (None, ']') => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The operator `word` outside quotes, parentheses and brackets; XPath lets a quote or a parenthesis
/// stand next to it with no space.
fn split_on<'a>(predicate: &'a str, word: &str) -> Vec<&'a str> {
    let bytes = predicate.as_bytes();
    let before = |at: usize| {
        at.checked_sub(1).is_some_and(|at| {
            matches!(
                bytes[at],
                b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b')'
            )
        })
    };
    let after = |at: usize| {
        bytes
            .get(at)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b'('))
    };
    let (mut parts, mut start, mut quote, mut depth) = (Vec::new(), 0, None, 0usize);
    for (at, c) in predicate.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth = depth.saturating_sub(1),
            (None, _)
                if depth == 0
                    && predicate[at..].starts_with(word)
                    && before(at)
                    && after(at + word.len()) =>
            {
                parts.push(&predicate[start..at]);
                start = at + word.len();
            }
            _ => {}
        }
    }
    parts.push(&predicate[start..]);
    parts
}

fn name_test(test: &str) -> Option<Test> {
    let test = test.trim();
    let (make, value): (fn(String) -> Test, &str) = match test.strip_prefix("defName") {
        Some(value) => (Test::DefName, value),
        None => match test.strip_prefix("@ParentName") {
            Some(value) => (Test::ParentName, value),
            None => (Test::Name, test.strip_prefix("@Name")?),
        },
    };
    let value = value.trim_start().strip_prefix('=')?.trim_start();
    let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let name = value.strip_prefix(quote)?.strip_suffix(quote)?;
    (!name.contains(quote)).then(|| make(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::super::Target;
    use super::super::document::Document;
    use super::super::{DefKey, ModFiles};
    use super::{indexed, select};
    use crate::rimworld::loadfolders;
    use libxml::xpath::Context;
    use std::fs;

    fn document_of(xml: &str) -> (tempfile::TempDir, Document) {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Defs")).unwrap();
        fs::write(tmp.path().join("Defs/a.xml"), xml).unwrap();
        let folders = loadfolders::folders(tmp.path(), "1.7", &|_| false);
        let document = Document::load(&[ModFiles { folders }]);
        (tmp, document)
    }

    const DEFS: &str = "<Defs>\
        <ThingDef><defName>A</defName><soundWorking>Y</soundWorking></ThingDef>\
        <ThingDef><defName>A</defName><soundWorking>N</soundWorking></ThingDef>\
        <ThingDef><defName>B</defName><soundWorking>Y</soundWorking></ThingDef>\
        <ThingDef><defName>C</defName><soundWorking>Y</soundWorking></ThingDef>\
        <ThingDef Name=\"BaseX\"><soundWorking>Y</soundWorking></ThingDef>\
        <GeneDef><defName>A</defName><soundWorking>Y</soundWorking></GeneDef>\
        </Defs>";

    fn keys_of(document: &Document, nodes: &[libxml::tree::Node]) -> Vec<(Option<DefKey>, String)> {
        nodes
            .iter()
            .map(|node| (document.def_of(node), node.get_content()))
            .collect()
    }

    fn the_index_matches_libxml2(paths: &[&str]) {
        let (_tmp, document) = document_of(DEFS);
        let context = Context::new(document.tree()).unwrap();
        for path in paths {
            assert!(indexed(path).is_some(), "{path}");
            let from = document.root().get_parent().unwrap();
            let plain: Vec<_> = context
                .node_evaluate(path, &from)
                .unwrap()
                .get_nodes_as_vec();
            let through = select(&document, &context, path).ok().unwrap().nodes;
            assert_eq!(
                keys_of(&document, &through),
                keys_of(&document, &plain),
                "{path}"
            );
        }
    }

    #[test]
    fn a_leading_star_is_the_defs_root() {
        let paths = [
            "*/ThingDef[defName=\"A\"]",
            "/*/ThingDef[defName=\"A\" or defName=\"B\"]/soundWorking",
        ];
        for path in paths {
            let step = indexed(path).expect(path);
            assert_eq!(step.kind, "ThingDef", "{path}");
        }
        the_index_matches_libxml2(&paths);
        for star in ["*/ThingDef[defName=\"A\"]", "/*/ThingDef[defName=\"A\"]"] {
            let target = Target::of_xpath(star);
            assert_eq!(target, Target::of_xpath("Defs/ThingDef[defName=\"A\"]"));
        }
        let wide = Target::of_xpath("*/ThingDef/comps");
        assert_eq!((wide.kind(), wide.names()), (Some("ThingDef"), None));
        assert_eq!(Target::of_xpath("*/*/ThingDef").kind(), None);
        assert_eq!(Target::of_xpath("//*/ThingDef").kind(), None);
    }

    #[test]
    fn the_index_follows_and_like_libxml2() {
        the_index_matches_libxml2(&[
            "Defs/ThingDef[defName=\"A\" and soundWorking=\"Y\"]",
            "Defs/ThingDef[soundWorking=\"Y\" and defName=\"A\"]/soundWorking",
            "Defs/ThingDef[(defName=\"A\" or defName=\"B\") and soundWorking=\"Y\"]",
            "Defs/ThingDef[(defName=\"A\" or defName=\"B\")]",
            "Defs/ThingDef[defName=\"A\" and defName=\"B\"]",
            "Defs/ThingDef[(defName=\"A\" or defName=\"B\") and (defName=\"B\" or defName=\"C\")]",
            "Defs/ThingDef[defName=\"A\" and soundWorking=\"Y\" or defName=\"B\"]",
            "Defs/ThingDef[@Name=\"BaseX\" and soundWorking=\"Y\"]",
            "Defs/ThingDef[defName=\"A\" and soundWorking[.=\"Y\"]]",
        ]);
        for wide in [
            "Defs/ThingDef[@ParentName=\"BaseX\"]",
            "Defs/ThingDef[defName=\"A\" or soundWorking=\"Y\"]",
            "Defs/ThingDef[soundWorking=\"Y\" and soundWorking=\"Y\"]",
            "Defs/ThingDef[defName=\"A\" and soundWorking=\"Y\" or soundWorking=\"Y\"]",
        ] {
            assert!(indexed(wide).is_none(), "{wide}");
        }
    }

    #[test]
    fn the_index_reads_spaced_and_joined_names() {
        for (path, want) in [
            (
                "Defs/ThingDef[defName = \"T\" or defName=\"U\"]/x",
                vec!["T", "U"],
            ),
            (
                "Defs/ThingDef[defName=\"A\"or defName=\"B\"]",
                vec!["A", "B"],
            ),
        ] {
            let step = indexed(path).expect(path);
            let names: Vec<&str> = step.names.iter().map(|(_, n)| n.as_str()).collect();
            assert_eq!(names, want, "{path}");
        }
    }
}
