use super::*;
use crate::rimworld::loadfolders;
use crate::rimworld::patching::ModFiles;
use crate::rimworld::patching::document::Document;
use crate::rimworld::patchsettings::Settings;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const DEFS: &str = "<ThingDef><defName>A</defName><label>a</label></ThingDef>\
                    <ThingDef><defName>B</defName></ThingDef>";

struct Outcome {
    xml: String,
    traces: Vec<Trace>,
    failures: Vec<Failure>,
    unknown: Vec<Target>,
}

fn key(name: &str) -> DefKey {
    DefKey {
        kind: "ThingDef".into(),
        name: name.into(),
    }
}

fn active(ids: &[&str], names: &[&str]) -> Active {
    Active {
        ids: ids.iter().map(|s| s.to_string()).collect::<HashSet<_>>(),
        names: names.iter().map(|s| s.to_string()).collect::<HashSet<_>>(),
    }
}

fn op(class: &str, body: &str) -> String {
    format!("<Operation Class=\"{class}\">{body}</Operation>")
}

fn xpath_a(tail: &str) -> String {
    format!("<xpath>Defs/ThingDef[defName=\"A\"]{tail}</xpath>")
}

fn mod_files(root: &Path) -> ModFiles {
    ModFiles {
        folders: loadfolders::folders(root, "1.7", &|_| false),
    }
}

fn run_mods(defs: &str, patches: &[&str], active: &Active, order: &[usize]) -> Outcome {
    run_mods_with(defs, patches, active, &Settings::default(), order)
}

fn run_mods_with(
    defs: &str,
    patches: &[&str],
    active: &Active,
    settings: &Settings,
    order: &[usize],
) -> Outcome {
    let tmp = tempfile::tempdir().unwrap();
    let mut mods = Vec::new();
    for (at, patch) in patches.iter().enumerate() {
        let root = tmp.path().join(format!("mod{at}"));
        fs::create_dir_all(root.join("Patches")).unwrap();
        fs::create_dir_all(root.join("Defs")).unwrap();
        if at == 0 {
            fs::write(root.join("Defs/a.xml"), format!("<Defs>{defs}</Defs>")).unwrap();
        }
        fs::write(root.join("Patches/p.xml"), patch).unwrap();
        mods.push(mod_files(&root));
    }
    let mut document = Document::load(&mods);
    let patches = Patches::read(&mods, active, settings);
    let (mut unknown, mut traces, mut failures) = (Vec::new(), Vec::new(), Vec::new());
    apply(
        &mut document,
        &patches,
        order,
        &mut unknown,
        &mut traces,
        &mut failures,
    );
    let tree = document.tree();
    let xml = tree.node_to_string(&tree.get_root_element().unwrap());
    Outcome {
        xml,
        traces,
        failures,
        unknown,
    }
}

fn run(ops: &[String]) -> Outcome {
    run_with(ops, &active(&[], &[]))
}

fn run_with(ops: &[String], active: &Active) -> Outcome {
    let patch = format!("<Patch>{}</Patch>", ops.concat());
    run_mods(DEFS, &[&patch], active, &[0])
}

fn a_with(inside: &str) -> String {
    format!("<Defs>{inside}<ThingDef><defName>B</defName></ThingDef></Defs>")
}

#[test]
fn add_appends_and_prepends() {
    let appended = op(
        "PatchOperationAdd",
        &format!("{}<value><x/><y/></value>", xpath_a("")),
    );
    let prepended = op(
        "PatchOperationAdd",
        &format!(
            "{}<order>Prepend</order><value><p/><q/></value>",
            xpath_a("")
        ),
    );
    let out = run(&[appended, prepended]);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><p/><q/><defName>A</defName><label>a</label><x/><y/></ThingDef>")
    );
    assert!(out.failures.is_empty());
    assert_eq!(out.traces.len(), 2);
    assert_eq!(out.traces[0].wrote, BTreeSet::from([key("A")]));
}

#[test]
fn insert_before_and_after() {
    let before = op(
        "PatchOperationInsert",
        &format!("{}<value><i/></value>", xpath_a("/label")),
    );
    let after = op(
        "PatchOperationInsert",
        &format!(
            "{}<order>Append</order><value><j/></value>",
            xpath_a("/label")
        ),
    );
    let out = run(&[before, after]);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><i/><label>a</label><j/></ThingDef>")
    );
}

#[test]
fn remove() {
    let label = op("PatchOperationRemove", &xpath_a("/label"));
    let def = op(
        "PatchOperationRemove",
        "<xpath>Defs/ThingDef[defName=\"B\"]</xpath>",
    );
    let again = op(
        "PatchOperationAdd",
        "<xpath>Defs/ThingDef[defName=\"B\"]</xpath><value><x/></value>",
    );
    let out = run(&[label, def, again]);
    assert_eq!(
        out.xml,
        "<Defs><ThingDef><defName>A</defName></ThingDef></Defs>"
    );
    assert_eq!(out.failures.len(), 1, "the removed def is not found again");
    assert!(out.unknown.is_empty());
}

#[test]
fn replace() {
    let replace = op(
        "PatchOperationReplace",
        &format!("{}<value><name>z</name><n2/></value>", xpath_a("/label")),
    );
    let out = run(&[replace]);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><name>z</name><n2/></ThingDef>")
    );
}

#[test]
fn replacing_a_def_keeps_the_index_right() {
    let replace = op(
        "PatchOperationReplace",
        "<xpath>Defs/ThingDef[defName=\"B\"]</xpath>\
         <value><ThingDef><defName>C</defName></ThingDef></value>",
    );
    let then_c = op(
        "PatchOperationAdd",
        "<xpath>Defs/ThingDef[defName=\"C\"]</xpath><value><x/></value>",
    );
    let then_b = op(
        "PatchOperationAdd",
        "<xpath>Defs/ThingDef[defName=\"B\"]</xpath><value><x/></value>",
    );
    let out = run(&[replace, then_c, then_b]);
    assert!(out.xml.contains("<defName>C</defName><x/>"), "{}", out.xml);
    assert_eq!(out.failures.len(), 1);
    assert_eq!(out.traces[0].wrote, BTreeSet::from([key("B"), key("C")]));
}

#[test]
fn adding_a_def_to_defs_is_found_by_name() {
    let add = op(
        "PatchOperationAdd",
        "<xpath>Defs</xpath><value><ThingDef><defName>N</defName></ThingDef></value>",
    );
    let then = op(
        "PatchOperationAdd",
        "<xpath>Defs/ThingDef[defName=\"N\"]</xpath><value><x/></value>",
    );
    let out = run(&[add, then]);
    assert!(out.failures.is_empty());
    assert!(
        out.xml
            .ends_with("<defName>N</defName><x/></ThingDef></Defs>")
    );
    assert!(out.traces[0].wrote.contains(&key("N")));
}

#[test]
fn a_changed_def_name_is_followed() {
    let rename = op(
        "PatchOperationReplace",
        &format!("{}<value><defName>C</defName></value>", xpath_a("/defName")),
    );
    let then = op(
        "PatchOperationRemove",
        "<xpath>Defs/ThingDef[defName=\"C\"]</xpath>",
    );
    let out = run(&[rename, then]);
    assert!(out.failures.is_empty());
    assert!(!out.xml.contains("<label>"));
}

#[test]
fn a_name_attribute_is_found_through_the_index() {
    let defs = "<ThingDef Name=\"Base\"><defName>A</defName></ThingDef>";
    let patch = format!(
        "<Patch>{}</Patch>",
        op(
            "PatchOperationAdd",
            "<xpath>Defs/ThingDef[@Name=\"Base\"]</xpath><value><x/></value>"
        )
    );
    let out = run_mods(defs, &[&patch], &active(&[], &[]), &[0]);
    assert!(out.failures.is_empty());
    assert!(out.xml.contains("<x/>"));
}

#[test]
fn attribute_add_set_remove() {
    let add = op(
        "PatchOperationAttributeAdd",
        &format!(
            "{}<attribute>Abstract</attribute><value>True</value>",
            xpath_a("")
        ),
    );
    let set = op(
        "PatchOperationAttributeSet",
        &format!(
            "{}<attribute>Abstract</attribute><value>False</value>",
            xpath_a("")
        ),
    );
    let set_new = op(
        "PatchOperationAttributeSet",
        &format!("{}<attribute>Name</attribute><value>N</value>", xpath_a("")),
    );
    let remove = op(
        "PatchOperationAttributeRemove",
        &format!("{}<attribute>Name</attribute>", xpath_a("")),
    );
    let out = run(&[add.clone(), set]);
    assert!(
        out.xml.contains("<ThingDef Abstract=\"False\">"),
        "{}",
        out.xml
    );
    let out = run(&[add, set_new, remove]);
    assert!(
        out.xml.contains("<ThingDef Abstract=\"True\">"),
        "{}",
        out.xml
    );
    assert!(out.failures.is_empty());
}

#[test]
fn attribute_add_fails_when_every_match_has_it() {
    let add = op(
        "PatchOperationAttributeAdd",
        &format!(
            "{}<attribute>Abstract</attribute><value>True</value>",
            xpath_a("")
        ),
    );
    let remove = op(
        "PatchOperationAttributeRemove",
        &format!("{}<attribute>Nope</attribute>", xpath_a("")),
    );
    let out = run(&[add.clone(), add, remove]);
    assert_eq!(out.failures.len(), 2);
}

#[test]
fn add_mod_extension_creates_the_list() {
    let add = op(
        "PatchOperationAddModExtension",
        &format!("{}<value><li Class=\"X\"/></value>", xpath_a("")),
    );
    let again = op(
        "PatchOperationAddModExtension",
        &format!("{}<value><li Class=\"Y\"/></value>", xpath_a("")),
    );
    let out = run(&[add, again]);
    assert_eq!(
        out.xml,
        a_with(
            "<ThingDef><defName>A</defName><label>a</label>\
             <modExtensions><li Class=\"X\"/><li Class=\"Y\"/></modExtensions></ThingDef>"
        )
    );
}

#[test]
fn set_name() {
    let rename = op(
        "PatchOperationSetName",
        &format!("{}<name>title</name>", xpath_a("/label")),
    );
    let out = run(&[rename]);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><title>a</title></ThingDef>")
    );
}

#[test]
fn set_name_on_a_def_is_found_under_its_new_kind() {
    let rename = op(
        "PatchOperationSetName",
        "<xpath>Defs/ThingDef[defName=\"B\"]</xpath><name>PawnKindDef</name>",
    );
    let then = op(
        "PatchOperationAdd",
        "<xpath>Defs/PawnKindDef[defName=\"B\"]</xpath><value><x/></value>",
    );
    let out = run(&[rename, then]);
    assert!(out.failures.is_empty());
}

#[test]
fn sequence_stops_at_the_first_failure_and_fails() {
    let sequence = op(
        "PatchOperationSequence",
        &format!(
            "<operations>{}{}{}</operations>",
            op(
                "PatchOperationAdd",
                &format!("{}<value><x/></value>", xpath_a(""))
            ),
            op(
                "PatchOperationRemove",
                "<xpath>Defs/ThingDef[defName=\"zzz\"]</xpath>"
            ),
            op(
                "PatchOperationAdd",
                "<xpath>Defs/ThingDef[defName=\"B\"]</xpath><value><y/></value>"
            ),
        )
        .replace("<Operation", "<li")
        .replace("</Operation>", "</li>"),
    );
    let out = run(&[sequence]);
    assert!(out.xml.contains("<x/>"));
    assert!(!out.xml.contains("<y/>"));
    assert_eq!(out.failures.len(), 1);
    let Failure::Failed {
        mod_at,
        operation,
        xpath,
        target,
        ..
    } = &out.failures[0]
    else {
        panic!("a failed operation")
    };
    assert_eq!(*mod_at, 0);
    assert_eq!(
        operation,
        "Verse.PatchOperationSequence(count=3, lastFailedOperation=\
         Verse.PatchOperationRemove(Defs/ThingDef[defName=\"zzz\"]))"
    );
    assert_eq!(xpath, "Defs/ThingDef[defName=\"zzz\"]");
    assert_eq!(
        target.names().map(|n| n.iter().next().map(String::as_str)),
        Some(Some("zzz"))
    );
}

fn branch(tag: &str, class: &str, body: &str) -> String {
    format!("<{tag} Class=\"{class}\">{body}</{tag}>")
}

#[test]
fn conditional_takes_the_branch_and_does_not_fail_without_one() {
    let present = xpath_a("");
    let absent = "<xpath>Defs/ThingDef[defName=\"zzz\"]</xpath>";
    let add =
        |name: &str| format!("<xpath>Defs/ThingDef[defName=\"{name}\"]</xpath><value><m/></value>");
    let matched = op(
        "PatchOperationConditional",
        &format!(
            "{present}{}",
            branch("match", "PatchOperationAdd", &add("B"))
        ),
    );
    let unmatched = op(
        "PatchOperationConditional",
        &format!(
            "{absent}{}",
            branch("nomatch", "PatchOperationAdd", &add("A"))
        ),
    );
    let neither_taken = op(
        "PatchOperationConditional",
        &format!(
            "{absent}{}",
            branch("match", "PatchOperationAdd", &add("A"))
        ),
    );
    let out = run(&[matched, unmatched, neither_taken]);
    assert!(out.failures.is_empty());
    assert_eq!(out.xml.matches("<m/>").count(), 2);
}

#[test]
fn conditional_with_no_branches_fails() {
    let none = op("PatchOperationConditional", &xpath_a(""));
    let out = run(&[none]);
    assert_eq!(out.failures.len(), 1);
}

#[test]
fn find_mod_matches_by_name_among_the_active() {
    let add = "<xpath>Defs/ThingDef[defName=\"A\"]</xpath><value><m/></value>";
    let find = |mods: &str| {
        op(
            "PatchOperationFindMod",
            &format!(
                "<mods><li>{mods}</li></mods>{}{}",
                branch("match", "PatchOperationAdd", add),
                branch("nomatch", "PatchOperationAdd", &add.replace("<m/>", "<n/>")),
            ),
        )
    };
    let on = active(&["cool.id"], &["Cool Mod"]);
    let out = run_with(&[find("Cool Mod")], &on);
    assert!(out.xml.contains("<m/>") && !out.xml.contains("<n/>"));
    let out = run_with(&[find("cool.id")], &on);
    assert!(out.xml.contains("<n/>") && !out.xml.contains("<m/>"));
}

#[test]
fn find_mod_without_the_branch_succeeds() {
    let find = op("PatchOperationFindMod", "<mods><li>Nobody</li></mods>");
    let out = run(&[find]);
    assert!(out.failures.is_empty());
}

#[test]
fn test_succeeds_on_a_match() {
    let hit = op("PatchOperationTest", &xpath_a(""));
    let miss = op(
        "PatchOperationTest",
        "<xpath>Defs/ThingDef[defName=\"zzz\"]</xpath>",
    );
    let out = run(&[hit, miss]);
    assert_eq!(out.failures.len(), 1);
    assert!(out.traces[0].read.contains(&key("A")));
}

#[test]
fn success_modes() {
    let with = |class: &str, xpath: &str, success: &str| {
        op(
            class,
            &format!("<xpath>{xpath}</xpath><success>{success}</success>"),
        )
    };
    let miss = "Defs/ThingDef[defName=\"zzz\"]";
    let hit = "Defs/ThingDef[defName=\"A\"]";
    let out = run(&[
        with("PatchOperationTest", miss, "Always"),
        with("PatchOperationTest", hit, "Never"),
        with("PatchOperationTest", miss, "Invert"),
        with("PatchOperationTest", hit, "Invert"),
        with("PatchOperationTest", hit, "Normal"),
    ]);
    assert_eq!(out.failures.len(), 2);
    let Failure::Failed { operation, .. } = &out.failures[0] else {
        panic!("a failed operation")
    };
    assert!(operation.starts_with("Verse.PatchOperationTest"));
}

#[test]
fn an_unknown_class_marks_its_target() {
    let unknown = op(
        "XmlExtensions.PatchOperationMath",
        &format!("{}<value><x/></value>", xpath_a("/comps")),
    );
    let out = run(&[unknown]);
    assert!(out.failures.is_empty());
    assert_eq!(out.unknown.len(), 1);
    assert_eq!(out.unknown[0].kind(), Some("ThingDef"));
    assert_eq!(
        out.unknown[0]
            .names()
            .map(|n| n.iter().next().map(String::as_str)),
        Some(Some("A"))
    );
    assert_eq!(
        out.xml,
        a_with(&format!(
            "<ThingDef>{}</ThingDef>",
            "<defName>A</defName><label>a</label>"
        ))
    );
}

#[test]
fn an_unknown_wrapper_marks_only_what_its_inner_xpaths_aim_at() {
    let wrapper = op(
        "XmlExtensions.OptionalPatchExternal",
        &format!(
            "<caseTrue Class=\"PatchOperationAdd\">{}<value><x/></value></caseTrue>",
            xpath_a("/comps")
        ),
    );
    let out = run(&[wrapper]);
    assert_eq!(out.unknown.len(), 1);
    assert_eq!(
        out.unknown[0]
            .names()
            .map(|n| n.iter().next().map(String::as_str)),
        Some(Some("A"))
    );
}

#[test]
fn an_unknown_operation_without_any_xpath_reaches_nothing() {
    let setting = op("XmlExtensions.Setting.Text", "<text>hi</text>");
    let out = run(&[setting]);
    assert!(out.unknown.is_empty());
}

#[test]
fn an_unknown_operation_reaches_every_element_named_like_an_xpath_at_any_depth() {
    let wrapper = op(
        "XmlExtensions.OptionalPatchExternal",
        "<inner><xpathLocal>Defs/ThingDef[defName=\"B\"]</xpathLocal><deeper>\
         <sourceXPath>Defs/ThingDef[defName=\"A\"]</sourceXPath><texPath>Defs/X</texPath>\
         </deeper></inner>",
    );
    let out = run(&[wrapper]);
    let names: Vec<_> = out
        .unknown
        .iter()
        .map(|t| t.names().and_then(|n| n.iter().next().map(String::as_str)))
        .collect();
    assert_eq!(names, [Some("B"), Some("A")]);
}

#[test]
fn a_refused_xpath_marks_its_target() {
    let refused = op(
        "PatchOperationAdd",
        "<xpath>Defs/ThingDef[defName=\"A\"</xpath><value><x/></value>",
    );
    let out = run(&[refused]);
    assert!(out.failures.is_empty());
    assert_eq!(out.unknown.len(), 1);
}

#[test]
fn may_require_drops_a_sequence_li_but_keeps_value_lis() {
    let sequence = "<Operation Class=\"PatchOperationSequence\"><operations>\
        <li Class=\"PatchOperationAdd\" MayRequire=\"gone.mod\">\
        <xpath>Defs/ThingDef[defName=\"A\"]</xpath><value><dropped/></value></li>\
        <li Class=\"PatchOperationAdd\" MayRequire=\"here.mod\">\
        <xpath>Defs/ThingDef[defName=\"A\"]</xpath><value><kept/></value></li>\
        <li Class=\"PatchOperationAdd\">\
        <xpath>Defs/ThingDef[defName=\"B\"]</xpath>\
        <value><li MayRequireAnyOf=\"gone.mod, HERE.mod\">one</li>\
        <li MayRequire=\"gone.mod\">two</li></value></li>\
        </operations></Operation>";
    let patch = format!("<Patch>{sequence}</Patch>");
    let out = run_mods(DEFS, &[&patch], &active(&["here.mod"], &[]), &[0]);
    assert!(out.failures.is_empty());
    assert!(out.xml.contains("<kept/>") && !out.xml.contains("<dropped/>"));
    assert!(out.xml.contains(">one<"), "{}", out.xml);
    assert!(out.xml.contains("<li MayRequire=\"gone.mod\">two</li>"));
}

#[test]
fn traces_name_the_defs_read_and_written() {
    let conditional = op(
        "PatchOperationConditional",
        &format!(
            "{}{}",
            xpath_a(""),
            branch(
                "match",
                "PatchOperationAdd",
                "<xpath>Defs/ThingDef[defName=\"B\"]</xpath><value><x/></value>"
            )
        ),
    );
    let out = run(&[conditional]);
    assert_eq!(out.traces.len(), 1);
    let trace = &out.traces[0];
    assert_eq!(trace.read, BTreeSet::from([key("A"), key("B")]));
    assert_eq!(trace.wrote, BTreeSet::from([key("B")]));
    assert_eq!(trace.mod_at, 0);
    assert!(trace.file.ends_with("p.xml"));
}

#[test]
fn an_unparsable_patch_file_is_skipped() {
    let broken = run_mods(DEFS, &["<Patch><Operation"], &active(&[], &[]), &[0]);
    assert!(broken.failures.is_empty() && broken.unknown.is_empty());
    assert!(broken.traces.is_empty());
    let other_root = run_mods(DEFS, &["<Defs/>"], &active(&[], &[]), &[0]);
    assert!(other_root.failures.is_empty() && other_root.unknown.is_empty());
    let not_an_operation = run_mods(DEFS, &["<Patch><Thing/></Patch>"], &active(&[], &[]), &[0]);
    assert!(not_an_operation.traces.is_empty());
}

#[test]
fn mods_apply_in_the_order_given() {
    let patch = |tag: &str| {
        format!(
            "<Patch>{}</Patch>",
            op(
                "PatchOperationAdd",
                &format!("{}<value><{tag}/></value>", xpath_a(""))
            )
        )
    };
    let (one, two) = (patch("one"), patch("two"));
    let out = run_mods(DEFS, &[&one, &two], &active(&[], &[]), &[1, 0]);
    assert!(out.xml.contains("<two/><one/>"), "{}", out.xml);
    let order: Vec<usize> = out.traces.iter().map(|t| t.mod_at).collect();
    assert_eq!(order, [1, 0]);
}

#[test]
fn a_wide_xpath_goes_through_libxml() {
    let add = op(
        "PatchOperationAdd",
        "<xpath>Defs/ThingDef</xpath><value><w/></value>",
    );
    let out = run(&[add]);
    assert_eq!(out.xml.matches("<w/>").count(), 2);
    assert_eq!(out.traces[0].wrote, BTreeSet::from([key("A"), key("B")]));
}

#[test]
fn text_values_join_the_text_beside_them() {
    let text = |class: &str, tail: &str, order: &str, value: &str| {
        op(
            class,
            &format!("{}{order}<value>{value}</value>", xpath_a(tail)),
        )
    };
    let out = run(&[
        text("PatchOperationAdd", "/label", "", "b"),
        text("PatchOperationAdd", "/label", "<order>Prepend</order>", "z"),
        text(
            "PatchOperationInsert",
            "/label",
            "<order>Append</order>",
            "!",
        ),
    ]);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><label>zab</label>!</ThingDef>")
    );
}

#[test]
fn a_def_named_by_a_path_that_matches_nothing_is_read() {
    let miss = op(
        "PatchOperationAdd",
        &format!("{}<value><x/></value>", xpath_a("/statBases")),
    );
    let out = run(&[miss]);
    assert_eq!(out.failures.len(), 1);
    assert_eq!(out.traces[0].read, BTreeSet::from([key("A")]));
    assert!(out.traces[0].wrote.is_empty());
}

mod frameworks;
mod xmlextensions;
