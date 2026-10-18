use document::Document;
use operations::apply;
use std::collections::BTreeSet;
use std::path::PathBuf;
use types::Types;

mod checks;
mod document;
mod inheritance;
mod operations;
pub mod types;
mod xpath;

pub use operations::{Active, Audit, Failure, Patches, Trace, XpathSeen, audit};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefKey {
    pub kind: String,
    pub name: String,
}

/// What an XPath is known to reach among the top-level defs; `None` is any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub kind: Option<String>,
    pub names: Option<BTreeSet<String>>,
}

impl Target {
    pub fn of_xpath(xpath: &str) -> Target {
        let any = Target {
            kind: None,
            names: None,
        };
        let path = xpath.trim();
        if path.contains('|') || path.starts_with("//") || leaves_the_def(path) {
            return any;
        }
        let Some(below) = xpath::below_defs(path.trim_start_matches('/')) else {
            return any;
        };
        let end = below.find(['[', '/']).unwrap_or(below.len());
        let kind = below[..end].trim_end();
        if kind.is_empty()
            || kind.contains(['*', '(', '@', ':'])
            || kind.contains(char::is_whitespace)
        {
            return any;
        }
        Target {
            kind: Some(kind.to_string()),
            names: name_predicate(below[end..].trim_start()),
        }
    }

    pub fn covers(&self, def: &DefKey) -> bool {
        self.kind.as_ref().is_none_or(|kind| *kind == def.kind)
            && self
                .names
                .as_ref()
                .is_none_or(|names| names.contains(&def.name))
    }
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

/// The mods' defs and patches as read, ready to be played in any order.
pub struct Loaded {
    document: Document,
    patches: Patches,
    types: Types,
    active: Active,
    places: usize,
    replays: std::sync::atomic::AtomicU64,
}

/// What one play of the patches in one order gave.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub failures: Vec<Failure>,
    pub traces: Vec<Trace>,
    pub unknown: Vec<Target>,
}

impl Run {
    pub fn is_unknown(&self, def: &DefKey) -> bool {
        self.unknown.iter().any(|target| target.covers(def))
    }
}

static SUBSETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SUBSET_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Runs limited to some defs so far in this process and the time they took, for the timing test.
pub fn subset_runs() -> (u64, std::time::Duration) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        SUBSETS.load(Relaxed),
        std::time::Duration::from_nanos(SUBSET_NANOS.load(Relaxed)),
    )
}

impl Loaded {
    pub fn read(mods: Vec<ModFiles>, active: &Active, types: Types) -> Loaded {
        Loaded {
            document: Document::load(&mods),
            patches: Patches::read(&mods, active),
            types,
            active: active.clone(),
            places: mods.len(),
            replays: Default::default(),
        }
    }

    /// How many operations of each class RimStack does not apply, for the timing test.
    pub fn unknown_classes(&self) -> std::collections::BTreeMap<String, usize> {
        self.patches.unknown_classes()
    }

    pub fn def_count(&self) -> usize {
        self.document.len()
    }

    /// Runs limited to some defs that this `Loaded` has played.
    pub fn replays(&self) -> u64 {
        self.replays.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `order` lists places in `mods` in load order; a mod left out contributes nothing. With `only`, just
    /// those defs and the parents they inherit from are played.
    pub fn run(&self, order: &[usize], only: Option<&BTreeSet<DefKey>>) -> Run {
        let started = only.map(|_| std::time::Instant::now());
        if only.is_some() {
            self.replays
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let run = self.play(order, only);
        if let Some(started) = started {
            use std::sync::atomic::Ordering::Relaxed;
            SUBSETS.fetch_add(1, Relaxed);
            SUBSET_NANOS.fetch_add(started.elapsed().as_nanos() as u64, Relaxed);
        }
        run
    }

    fn play(&self, order: &[usize], only: Option<&BTreeSet<DefKey>>) -> Run {
        let mut rank = vec![None; self.places];
        for (position, &place) in order.iter().enumerate() {
            if let Some(slot) = rank.get_mut(place) {
                *slot = Some(position);
            }
        }
        let mut document = match only {
            Some(keep) => self.document.subset(keep),
            None => self.document.copy(),
        };
        document.arrange(&rank);
        let mut run = Run::default();
        apply(
            &mut document,
            &self.patches,
            order,
            &mut run.unknown,
            &mut run.traces,
            &mut run.failures,
        );
        run.failures
            .extend(checks::find(&document, &rank, &self.active, &self.types));
        run
    }
}

/// One per place in the load order: the folders `loadfolders::folders` gives, lowest priority first.
pub struct ModFiles {
    pub folders: Vec<PathBuf>,
}

/// The `Defs` and `Patches` files of one mod that the simulation reads, as it walks them.
pub fn files_read(files: &ModFiles) -> Vec<PathBuf> {
    let mut found = document::files_in_game_order(files, "Defs");
    found.extend(document::files_in_game_order(files, "Patches"));
    found
}

#[cfg(test)]
mod tests {
    use super::document::Document;
    use super::types::Types;
    use super::*;
    use crate::rimworld::dotnet::FieldType;
    use crate::rimworld::loadfolders;
    use libxml::parser::Parser;
    use libxml::xpath::Context;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    #[test]
    fn libxml2_evaluates_from_the_document_node() {
        let doc = Parser::default()
            .parse_string("<Defs><ThingDef><defName>A</defName></ThingDef></Defs>")
            .unwrap();
        let context = Context::new(&doc).unwrap();
        let document = doc.get_root_element().unwrap().get_parent().unwrap();
        let found = context.node_evaluate("Defs/ThingDef", &document).unwrap();
        assert_eq!(found.get_nodes_as_vec().len(), 1);
    }

    fn key(kind: &str, name: &str) -> DefKey {
        DefKey {
            kind: kind.into(),
            name: name.into(),
        }
    }

    fn write(root: &Path, file: &str, xml: &str) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, xml).unwrap();
    }

    fn defs(body: &str) -> String {
        format!("<Defs>{body}</Defs>")
    }

    fn files_of(root: &Path, active: &[&str]) -> ModFiles {
        let folders = loadfolders::folders(root, "1.7", &|id| active.contains(&id));
        ModFiles { folders }
    }

    #[test]
    fn target_reads_a_def_name() {
        let named = Target::of_xpath("Defs/XenotypeDef[defName=\"VU_ReturnedSkeletal\"]/genes");
        assert_eq!(named.kind.as_deref(), Some("XenotypeDef"));
        assert_eq!(names(&named), Some(vec!["VU_ReturnedSkeletal"]));

        let spaced = Target::of_xpath("Defs/ThingDef\n\t[defName=\"A\"]");
        assert_eq!(spaced, Target::of_xpath("Defs/ThingDef[defName=\"A\"]"));
        let kind = Target::of_xpath("/Defs/ThingDef/comps");
        assert_eq!(
            (kind.kind.as_deref(), names(&kind)),
            (Some("ThingDef"), None)
        );

        for wide in ["//li", "Defs/*", "Defs/ThingDef | Defs/PawnKindDef", "li"] {
            let target = Target::of_xpath(wide);
            assert_eq!((target.kind.as_ref(), target.names.as_ref()), (None, None));
            assert!(target.covers(&key("ThingDef", "A")), "{wide}");
        }
        assert!(named.covers(&key("XenotypeDef", "VU_ReturnedSkeletal")));
        assert!(!named.covers(&key("XenotypeDef", "Other")));
        assert!(!kind.covers(&key("GeneDef", "A")));
    }

    fn names(target: &Target) -> Option<Vec<&str>> {
        target
            .names
            .as_ref()
            .map(|set| set.iter().map(String::as_str).collect())
    }

    #[test]
    fn target_reads_names_joined_by_or() {
        let target = Target::of_xpath("Defs/ThingDef[defName=\"A\" or defName='B']/comps");
        assert_eq!(target.kind.as_deref(), Some("ThingDef"));
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
        assert_eq!(word.names, None);
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
            assert_eq!(target.kind.as_deref(), Some("ThingDef"), "{mixed}");
            assert_eq!(target.names, None, "{mixed}");
            assert!(target.covers(&key("ThingDef", "C")), "{mixed}");
        }
        let other = Target::of_xpath("Defs/ThingDef[defName=\"A\" or li]");
        assert_eq!(other.names, None);
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
        assert_eq!(mixed.names, None);
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
            assert_eq!(target.kind.as_deref(), Some("ThingDef"), "{wide}");
            assert_eq!(target.names, None, "{wide}");
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
            assert_eq!(
                (target.kind.as_ref(), target.names.as_ref()),
                (None, None),
                "{out}"
            );
        }
    }

    #[test]
    fn defs_load_in_list_order_with_their_place() {
        let tmp = tempfile::tempdir().unwrap();
        let (one, two) = (tmp.path().join("one"), tmp.path().join("two"));
        let def = defs("<ThingDef><defName>A</defName></ThingDef>");
        write(&one, "Defs/a.xml", &def);
        write(&two, "Defs/b.xml", &def);
        let document = Document::load(&[files_of(&one, &[]), files_of(&two, &[])]);
        let nodes = document.nodes_of(&key("ThingDef", "A"));
        assert_eq!(nodes.len(), 2);
        let places: Vec<usize> = nodes
            .iter()
            .map(|node| document.origin_of(node).unwrap().place)
            .collect();
        assert_eq!(places, [0, 1]);
        let file = &document.origin_of(&nodes[1]).unwrap().file;
        assert!(file.ends_with("b.xml"));
        let inside = nodes[1].get_first_element_child().unwrap();
        assert_eq!(document.def_of(&inside), Some(key("ThingDef", "A")));
    }

    #[test]
    fn a_name_attribute_is_a_key() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "Defs/a.xml",
            &defs("<ThingDef Name=\"BaseX\" Abstract=\"True\"/>"),
        );
        let document = Document::load(&[files_of(tmp.path(), &[])]);
        assert_eq!(document.nodes_of(&key("ThingDef", "BaseX")).len(), 1);
    }

    #[test]
    fn an_if_mod_active_folder_loads_only_with_its_mod() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(
            root,
            "LoadFolders.xml",
            "<loadFolders><default><li>/</li><li IfModActive=\"other.mod\">Extra</li></default></loadFolders>",
        );
        write(
            root,
            "Defs/a.xml",
            &defs("<ThingDef><defName>A</defName></ThingDef>"),
        );
        write(
            root,
            "Extra/Defs/b.xml",
            &defs("<ThingDef><defName>B</defName></ThingDef>"),
        );
        let b = key("ThingDef", "B");
        let without = Document::load(&[files_of(root, &[])]);
        assert!(without.nodes_of(&b).is_empty());
        let with = Document::load(&[files_of(root, &["other.mod"])]);
        assert_eq!(with.nodes_of(&b).len(), 1);
    }

    #[test]
    fn a_specific_folder_hides_the_same_file_below() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(
            root,
            "Defs/A.xml",
            &defs("<ThingDef><defName>Old</defName></ThingDef>"),
        );
        write(
            root,
            "1.7/Defs/A.xml",
            &defs("<ThingDef><defName>New</defName></ThingDef>"),
        );
        write(
            root,
            "Defs/.hidden.xml",
            &defs("<ThingDef><defName>Hidden</defName></ThingDef>"),
        );
        write(
            root,
            "Defs/Z.xml",
            &defs("<ThingDef><defName>Z</defName></ThingDef>"),
        );
        let document = Document::load(&[files_of(root, &[])]);
        assert_eq!(document.nodes_of(&key("ThingDef", "New")).len(), 1);
        assert!(document.nodes_of(&key("ThingDef", "Old")).is_empty());
        assert!(document.nodes_of(&key("ThingDef", "Hidden")).is_empty());
        assert_eq!(document.nodes_of(&key("ThingDef", "Z")).len(), 1);
    }

    #[test]
    fn a_file_that_does_not_parse_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "Defs/a.xml", "<Defs><ThingDef>");
        write(
            tmp.path(),
            "Defs/b.xml",
            &defs("<ThingDef><defName>B</defName></ThingDef>"),
        );
        let document = Document::load(&[files_of(tmp.path(), &[])]);
        assert_eq!(document.nodes_of(&key("ThingDef", "B")).len(), 1);
    }

    #[test]
    fn a_namespaced_file_does_not_stop_the_other_defs() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "Defs/a.xml",
            "<Defs xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
             <ThingDef xsi:type=\"x\"><defName>N</defName><xsi:odd/></ThingDef></Defs>",
        );
        write(
            tmp.path(),
            "Defs/b.xml",
            &defs("<ThingDef><defName>B</defName></ThingDef>"),
        );
        let document = Document::load(&[files_of(tmp.path(), &[])]);
        assert_eq!(document.nodes_of(&key("ThingDef", "B")).len(), 1);
        let keep = BTreeSet::from([key("ThingDef", "N"), key("ThingDef", "B")]);
        let subset = document.subset(&keep);
        assert_eq!(subset.nodes_of(&key("ThingDef", "B")).len(), 1);
    }

    #[test]
    fn a_subset_keeps_every_parent_of_the_name_whatever_its_kind() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "Defs/a.xml",
            &defs(
                "<ThingDef Name=\"A\" Abstract=\"True\"/>\
                 <PawnKindDef Name=\"A\" Abstract=\"True\"/>\
                 <ThingDef Name=\"A\" Abstract=\"True\"/>\
                 <ThingDef ParentName=\"A\"><defName>C</defName></ThingDef>",
            ),
        );
        let document = Document::load(&[files_of(tmp.path(), &[])]);
        let subset = document.subset(&BTreeSet::from([key("ThingDef", "C")]));
        assert_eq!(subset.len(), 4);
    }

    #[test]
    fn a_subset_keeps_the_parent_chain() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "Defs/a.xml",
            &defs(
                "<ThingDef Name=\"A\" Abstract=\"True\"/>\
                 <ThingDef Name=\"B\" ParentName=\"A\" Abstract=\"True\"/>\
                 <ThingDef ParentName=\"B\"><defName>C</defName></ThingDef>\
                 <ThingDef><defName>D</defName></ThingDef>",
            ),
        );
        let document = Document::load(&[files_of(tmp.path(), &[])]);
        let keep = BTreeSet::from([key("ThingDef", "C")]);
        let subset = document.subset(&keep);
        for kept in ["A", "B", "C"] {
            assert_eq!(subset.nodes_of(&key("ThingDef", kept)).len(), 1, "{kept}");
        }
        assert!(subset.nodes_of(&key("ThingDef", "D")).is_empty());
        let origin = subset
            .origin_of(&subset.nodes_of(&key("ThingDef", "C"))[0])
            .unwrap();
        assert_eq!(origin.place, 0);
    }

    fn game_types() -> Types {
        let list = FieldType::ListOf("Verse.GeneDef".into());
        Types::from_fields(&[
            ("Verse.Def", None, &[]),
            ("Verse.GeneDef", Some("Verse.Def"), &[]),
            (
                "RimWorld.XenotypeDef",
                Some("Verse.Def"),
                &[("genes", list)],
            ),
            (
                "Verse.ThingDef",
                Some("Verse.Def"),
                &[("comps", FieldType::Other)],
            ),
        ])
    }

    /// Each mod is its defs and its patch operations.
    fn loaded(mods: &[(&str, &str)]) -> Loaded {
        let tmp = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        for (at, (defs_xml, patch)) in mods.iter().enumerate() {
            let root = tmp.path().join(format!("mod{at}"));
            if !defs_xml.is_empty() {
                write(&root, "Defs/d.xml", &defs(defs_xml));
            }
            if !patch.is_empty() {
                write(&root, "Patches/p.xml", &format!("<Patch>{patch}</Patch>"));
            }
            files.push(files_of(&root, &[]));
        }
        Loaded::read(files, &Active::default(), game_types())
    }

    const SKELETAL: &str = "<XenotypeDef><defName>VU_ReturnedSkeletal</defName>\
                            <genes><li>MaxTemp_HugeIncrease</li></genes></XenotypeDef>";
    const GENES: &str = "Defs/XenotypeDef[defName=\"VU_ReturnedSkeletal\"]/genes";

    fn reported_pair() -> Loaded {
        let add = format!(
            "<Operation Class=\"PatchOperationAdd\"><xpath>{GENES}</xpath>\
             <value><li>AG_HeatImmunity</li></value></Operation>"
        );
        let swap = format!(
            "<Operation Class=\"PatchOperationConditional\">\
             <xpath>{GENES}/li[text()=\"AG_HeatImmunity\"]</xpath>\
             <match Class=\"PatchOperationRemove\"><xpath>{GENES}/li[text()=\"MaxTemp_HugeIncrease\"]</xpath></match>\
             <nomatch Class=\"PatchOperationReplace\"><xpath>{GENES}/li[text()=\"MaxTemp_HugeIncrease\"]</xpath>\
             <value><li>AG_HeatImmunity</li></value></nomatch></Operation>"
        );
        loaded(&[(SKELETAL, ""), ("", &add), ("", &swap)])
    }

    fn twice(kind: &str, name: &str, field: &str, entry: &str) -> Failure {
        Failure::Twice {
            def: key(kind, name),
            field: field.into(),
            entry: entry.into(),
        }
    }

    fn doubled() -> Failure {
        twice(
            "XenotypeDef",
            "VU_ReturnedSkeletal",
            "genes",
            "AG_HeatImmunity",
        )
    }

    #[test]
    fn the_reported_pair_doubles_only_in_one_order() {
        let pair = reported_pair();
        assert_eq!(pair.run(&[0, 1, 2], None).failures, []);
        assert_eq!(pair.run(&[0, 2, 1], None).failures, [doubled()]);
    }

    #[test]
    fn a_replay_on_a_subset_matches_the_full_run() {
        let pair = reported_pair();
        let only = BTreeSet::from([key("XenotypeDef", "VU_ReturnedSkeletal")]);
        assert_eq!(pair.run(&[0, 2, 1], Some(&only)).failures, [doubled()]);
        assert_eq!(pair.run(&[0, 1, 2], Some(&only)).failures, []);
    }

    fn genes_child(own: &str) -> String {
        format!(
            "<XenotypeDef Name=\"P\" Abstract=\"True\"><genes><li>X</li></genes></XenotypeDef>\
             <XenotypeDef ParentName=\"P\"><defName>Kid</defName>{own}</XenotypeDef>"
        )
    }

    #[test]
    fn inheritance_doubles_a_list_entry() {
        let kid = twice("XenotypeDef", "Kid", "genes", "X");
        let appended = loaded(&[(&genes_child("<genes><li>X</li></genes>"), "")]);
        assert_eq!(appended.run(&[0], None).failures, [kid]);
        let replaced = genes_child("<genes Inherit=\"False\"><li>X</li></genes>");
        assert_eq!(loaded(&[(&replaced, "")]).run(&[0], None).failures, []);
    }

    #[test]
    fn a_list_of_unknown_type_may_repeat() {
        let thing = "<XenotypeDef><defName>A</defName>\
                     <mystery><li>X</li><li>X</li></mystery></XenotypeDef>";
        assert_eq!(loaded(&[(thing, "")]).run(&[0], None).failures, []);
    }

    #[test]
    fn a_list_entry_for_a_mod_that_is_not_active_does_not_count() {
        let thing = "<XenotypeDef><defName>A</defName><genes><li>X</li>\
                     <li MayRequire=\"not.here\">X</li></genes></XenotypeDef>\
                     <XenotypeDef MayRequire=\"not.here\"><defName>B</defName>\
                     <genes><li>X</li><li>X</li></genes></XenotypeDef>";
        assert_eq!(loaded(&[(thing, "")]).run(&[0], None).failures, []);
    }

    #[test]
    fn two_mods_adding_comps_to_a_child_def_duplicate() {
        let defs = "<ThingDef Name=\"Base\" Abstract=\"True\"/>\
                    <ThingDef ParentName=\"Base\"><defName>Kid</defName></ThingDef>\
                    <ThingDef><defName>Solo</defName></ThingDef>";
        let add = |name: &str| {
            format!(
                "<Operation Class=\"PatchOperationAdd\">\
                 <xpath>Defs/ThingDef[defName=\"{name}\"]</xpath>\
                 <value><comps><li>c</li></comps></value></Operation>"
            )
        };
        let mods = |name: &str| loaded(&[(defs, ""), ("", &add(name)), ("", &add(name))]);
        let duplicate = Failure::Duplicate {
            def: key("ThingDef", "Kid"),
            element: "comps".into(),
        };
        assert_eq!(mods("Kid").run(&[0, 1, 2], None).failures, [duplicate]);
        assert_eq!(mods("Solo").run(&[0, 1, 2], None).failures, []);
    }

    #[test]
    fn a_stat_named_twice_duplicates_even_without_a_parent() {
        let defs = "<ThingDef><defName>A</defName><statBases>\
                    <MarketValue>1</MarketValue><MarketValue>2</MarketValue></statBases></ThingDef>";
        let duplicate = Failure::Duplicate {
            def: key("ThingDef", "A"),
            element: "MarketValue".into(),
        };
        assert_eq!(loaded(&[(defs, "")]).run(&[0], None).failures, [duplicate]);
    }

    #[test]
    fn a_parent_is_chosen_by_load_order() {
        let base = |gene: &str| {
            format!(
                "<XenotypeDef Name=\"Base\" Abstract=\"True\"><genes><li>{gene}</li></genes></XenotypeDef>"
            )
        };
        let kid = "<XenotypeDef ParentName=\"Base\"><defName>Kid</defName>\
                   <genes><li>G2</li></genes></XenotypeDef>";
        let (first, second) = (base("G1"), base("G2"));
        let chain = loaded(&[(&first, ""), (&second, ""), (kid, "")]);
        let doubled = twice("XenotypeDef", "Kid", "genes", "G2");
        assert_eq!(chain.run(&[0, 1, 2], None).failures, [doubled]);
        assert_eq!(chain.run(&[1, 0, 2], None).failures, []);
    }

    #[test]
    fn an_unknown_operation_makes_the_defs_it_could_reach_unknown() {
        let op = "<Operation Class=\"Mod.Custom\"><xpath>Defs/XenotypeDef</xpath></Operation>";
        let run = loaded(&[(SKELETAL, op)]).run(&[0], None);
        assert!(run.is_unknown(&key("XenotypeDef", "VU_ReturnedSkeletal")));
        assert!(!run.is_unknown(&key("ThingDef", "A")));
    }
}
