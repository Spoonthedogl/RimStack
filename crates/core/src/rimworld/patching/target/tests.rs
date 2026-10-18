use super::*;
use crate::rimworld::loadfolders;
use crate::rimworld::patching::document::Document;
use crate::rimworld::patching::{DefKey, ModFiles};
use libxml::xpath::Context;

fn document_of(xml: &str) -> (tempfile::TempDir, Document) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("Defs")).unwrap();
    std::fs::write(tmp.path().join("Defs/a.xml"), xml).unwrap();
    let folders = loadfolders::folders(tmp.path(), "1.7", &|_| false);
    let document = Document::load(&[ModFiles { folders }]);
    (tmp, document)
}

/// The top-level defs libxml2 itself selects, or holds a selected node.
fn libxml_selects(document: &Document, xpath: &str) -> BTreeSet<DefKey> {
    let context = Context::new(document.tree()).unwrap();
    let from = document.root().get_parent().unwrap();
    let found = context.node_evaluate(xpath, &from).unwrap();
    let nodes = found.get_nodes_as_vec();
    nodes.iter().filter_map(|n| document.def_of(n)).collect()
}

fn covered(document: &Document, target: &Target) -> BTreeSet<DefKey> {
    let tops = document.tops();
    tops.into_iter()
        .filter_map(|top| top.key)
        .filter(|key| target.covers(key))
        .collect()
}

/// The target of the top-level step alone is what libxml2 selects; with more steps it holds every def
/// libxml2 selected something in.
fn matches_libxml(document: &Document, xpath: &str, rest: &str) {
    let target = Target::of_xpath(xpath);
    assert_eq!(
        covered(document, &target),
        libxml_selects(document, xpath),
        "{xpath}"
    );
    let longer = format!("{xpath}{rest}");
    let target = Target::of_xpath(&longer);
    let selected = libxml_selects(document, &longer);
    assert!(selected.is_subset(&covered(document, &target)), "{longer}");
}

const DEFS: &str = "<Defs>\
    <ThingDef><defName>A</defName><comps/></ThingDef>\
    <ThingDef><defName>B</defName><comps/></ThingDef>\
    <thingDef><defName>C</defName><comps/></thingDef>\
    <PawnKindDef><defName>A</defName><comps/></PawnKindDef>\
    <PawnKindDef><defName>D</defName><comps/></PawnKindDef>\
    <GeneDef><defName>G</defName><comps/></GeneDef>\
    </Defs>";

fn key(kind: &str, name: &str) -> DefKey {
    DefKey {
        kind: kind.into(),
        name: name.into(),
    }
}

#[test]
fn target_reads_a_def_name() {
    let named = Target::of_xpath("Defs/XenotypeDef[defName=\"VU_ReturnedSkeletal\"]/genes");
    assert_eq!(named.kind(), Some("XenotypeDef"));
    assert_eq!(names(&named), Some(vec!["VU_ReturnedSkeletal"]));

    let spaced = Target::of_xpath("Defs/ThingDef\n\t[defName=\"A\"]");
    assert_eq!(spaced, Target::of_xpath("Defs/ThingDef[defName=\"A\"]"));
    let kind = Target::of_xpath("/Defs/ThingDef/comps");
    assert_eq!((kind.kind(), names(&kind)), (Some("ThingDef"), None));

    for wide in ["//li", "Defs/*", "li"] {
        let target = Target::of_xpath(wide);
        assert_eq!((target.kind(), target.names()), (None, None));
        assert!(target.covers(&key("ThingDef", "A")), "{wide}");
    }
    assert!(named.covers(&key("XenotypeDef", "VU_ReturnedSkeletal")));
    assert!(!named.covers(&key("XenotypeDef", "Other")));
    assert!(!kind.covers(&key("GeneDef", "A")));
}

fn names(target: &Target) -> Option<Vec<&str>> {
    target
        .names()
        .map(|set| set.iter().map(String::as_str).collect())
}

#[test]
fn target_reads_names_joined_by_or() {
    let target = Target::of_xpath("Defs/ThingDef[defName=\"A\" or defName='B']/comps");
    assert_eq!(target.kind(), Some("ThingDef"));
    assert_eq!(names(&target), Some(vec!["A", "B"]));
    assert!(target.covers(&key("ThingDef", "A")));
    assert!(target.covers(&key("ThingDef", "B")));
    assert!(!target.covers(&key("ThingDef", "C")));
    assert!(!target.covers(&key("GeneDef", "A")));
}

#[test]
fn names_split_on_or_without_spaces() {
    let target = Target::of_xpath("Defs/ThingDef[defName=\"A\"or defName=\"B\"]/comps");
    assert_eq!(names(&target), Some(vec!["A", "B"]));
    let target = Target::of_xpath("Defs/ThingDef[defName='A' or\ndefName='B'or defName=\"C\"]");
    assert_eq!(names(&target), Some(vec!["A", "B", "C"]));
    let inside = Target::of_xpath("Defs/ThingDef[defName=\"Door or Floor\"]");
    assert_eq!(names(&inside), Some(vec!["Door or Floor"]));
    let word = Target::of_xpath("Defs/ThingDef[defName=\"A\" or Floor]");
    assert_eq!(word.names(), None);
}

#[test]
fn a_name_test_may_have_spaces_around_the_equals() {
    let target = Target::of_xpath("Defs/ThingDef[defName = \"Temperature\"]/x");
    assert_eq!(names(&target), Some(vec!["Temperature"]));
    let both = Target::of_xpath("Defs/ThingDef[defName =\"A\" or defName= 'B']");
    assert_eq!(names(&both), Some(vec!["A", "B"]));
}

#[test]
fn a_name_attribute_test_covers_a_def_that_also_has_a_defname() {
    let target = Target::of_xpath("Defs/ThingDef[@Name='BaseX']/comps");
    assert!(target.covers(&key("ThingDef", "KeyedByItsDefName")));
    assert!(!target.covers(&key("RecipeDef", "KeyedByItsDefName")));
}

#[test]
fn an_attribute_name_among_the_names_widens_them() {
    for mixed in [
        "Defs/ThingDef[defName=\"A\" or @Name=\"B\"]",
        "Defs/ThingDef[@Name=\"B\" or defName=\"A\"]/comps",
        "Defs/ThingDef[@Name=\"A\" or @Name=\"B\"]",
    ] {
        let target = Target::of_xpath(mixed);
        assert_eq!(target.kind(), Some("ThingDef"), "{mixed}");
        assert_eq!(target.names(), None, "{mixed}");
        assert!(target.covers(&key("ThingDef", "C")), "{mixed}");
    }
    let other = Target::of_xpath("Defs/ThingDef[defName=\"A\" or li]");
    assert_eq!(other.names(), None);
}

#[test]
fn names_joined_by_and_to_another_condition() {
    for (xpath, want) in [
        (
            "Defs/ThingDef[defName=\"A\" and soundWorking=\"Y\"]",
            vec!["A"],
        ),
        (
            "Defs/ThingDef[soundWorking=\"Y\" and defName=\"A\"]/x",
            vec!["A"],
        ),
        (
            "Defs/ThingDef[(defName=\"A\" or defName=\"B\") and soundWorking=\"Y\"]",
            vec!["A", "B"],
        ),
        (
            "Defs/ThingDef[(defName=\"A\" or defName=\"B\")]",
            vec!["A", "B"],
        ),
        (
            "Defs/ThingDef[(defName=\"A\" or defName=\"B\") and (defName=\"B\" or defName=\"C\")]",
            vec!["B"],
        ),
        ("Defs/ThingDef[defName=\"A\" and defName=\"A\"]", vec!["A"]),
        (
            "Defs/ThingDef[defName=\"A\" and soundWorking[.=\" or \"]]",
            vec!["A"],
        ),
        (
            "Defs/ThingDef[defName=\"A\" and x or defName=\"B\"]",
            vec!["A", "B"],
        ),
    ] {
        let target = Target::of_xpath(xpath);
        assert_eq!(names(&target), Some(want), "{xpath}");
    }
    let none = Target::of_xpath("Defs/ThingDef[defName=\"A\" and defName=\"B\"]");
    assert_eq!(names(&none), Some(vec![]));
    let mixed = Target::of_xpath("Defs/ThingDef[@Name=\"A\" and defName=\"B\"]");
    assert_eq!(mixed.names(), None);
}

#[test]
fn an_or_with_another_condition_stays_wide() {
    for wide in [
        "Defs/ThingDef[defName=\"A\" or soundWorking=\"Y\"]",
        "Defs/ThingDef[defName=\"A\" and x or soundWorking=\"Y\"]",
        "Defs/ThingDef[(defName=\"A\" or x) and y]",
        "Defs/ThingDef[soundWorking=\"Y\" and x]",
        "Defs/ThingDef[not(defName=\"A\")]",
    ] {
        let target = Target::of_xpath(wide);
        assert_eq!(target.kind(), Some("ThingDef"), "{wide}");
        assert_eq!(target.names(), None, "{wide}");
    }
}

#[test]
fn a_step_out_of_the_def_widens_the_target() {
    for out in [
        "Defs/ThingDef[defName=\"X\"]/../ThingDef[defName=\"Y\"]/comps",
        "Defs/ThingDef[defName=\"X\"]/parent::Defs/ThingDef",
        "Defs/ThingDef[defName=\"X\"]/ancestor::Defs",
        "Defs/ThingDef[defName=\"X\"]/ancestor-or-self::Defs",
        "Defs/ThingDef[defName=\"X\"]/following::ThingDef",
        "Defs/ThingDef[defName=\"X\"]/following-sibling::ThingDef",
        "Defs/ThingDef[defName=\"X\"]/preceding::ThingDef",
        "Defs/ThingDef[defName=\"X\"]/preceding-sibling::ThingDef",
    ] {
        let target = Target::of_xpath(out);
        assert_eq!((target.kind(), target.names()), (None, None), "{out}");
    }
}

#[test]
fn a_union_reaches_what_each_side_does() {
    let (_tmp, document) = document_of(DEFS);
    for xpath in [
        "(/Defs/thingDef | /Defs/ThingDef)",
        "Defs/ThingDef[defName=\"A\"] | Defs/PawnKindDef[defName='D']",
        "(Defs/ThingDef[defName=\"A\" or defName=\"B\"] | Defs/GeneDef)",
        "Defs/ThingDef[defName=\"A|B\"] | /Defs/GeneDef",
        "(Defs/ThingDef | Defs/PawnKindDef)[defName=\"A\"]",
    ] {
        matches_libxml(&document, xpath, "/comps");
    }
    let wide = Target::of_xpath("Defs/ThingDef[defName=\"A\"] | //li");
    assert!(wide.covers(&key("GeneDef", "G")));
}
