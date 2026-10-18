use super::*;
use crate::mods::sort;
use crate::rimworld::dotnet::FieldType;
use crate::rimworld::loadfolders;
use crate::rimworld::patching::types::Types;
use crate::rimworld::patching::{Active, ModFiles};
use std::fs;
use std::path::Path;

const X: &str = "VU_ReturnedSkeletal";
const Y: &str = "Y_Xeno";

fn write(root: &Path, file: &str, xml: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, xml).unwrap();
}

fn types() -> Types {
    let list = FieldType::ListOf("Verse.GeneDef".into());
    Types::from_fields(&[
        ("Verse.Def", None, &[]),
        ("Verse.GeneDef", Some("Verse.Def"), &[]),
        (
            "RimWorld.XenotypeDef",
            Some("Verse.Def"),
            &[("genes", list)],
        ),
    ])
}

/// Each mod is its defs and its patch operations.
fn loaded(mods: &[(String, String)]) -> (tempfile::TempDir, Loaded) {
    let tmp = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    for (at, (defs, patch)) in mods.iter().enumerate() {
        let root = tmp.path().join(format!("mod{at}"));
        if !defs.is_empty() {
            write(&root, "Defs/d.xml", &format!("<Defs>{defs}</Defs>"));
        }
        if !patch.is_empty() {
            write(&root, "Patches/p.xml", &format!("<Patch>{patch}</Patch>"));
        }
        let folders = loadfolders::folders(&root, "1.7", &|_| false);
        files.push(ModFiles { folders });
    }
    (tmp, Loaded::read(files, &Active::default(), types()))
}

fn xeno(name: &str, gene: &str) -> String {
    format!("<XenotypeDef><defName>{name}</defName><genes><li>{gene}</li></genes></XenotypeDef>")
}

fn genes(def: &str) -> String {
    format!("Defs/XenotypeDef[defName=\"{def}\"]/genes")
}

fn base_gene(def: &str) -> String {
    if def == X {
        "MaxTemp_HugeIncrease"
    } else {
        "Y_Base"
    }
    .into()
}

fn added(def: &str) -> String {
    if def == X {
        "AG_HeatImmunity"
    } else {
        "Y_Added"
    }
    .into()
}

fn add(def: &str) -> String {
    format!(
        "<Operation Class=\"PatchOperationAdd\"><xpath>{}</xpath>\
         <value><li>{}</li></value></Operation>",
        genes(def),
        added(def)
    )
}

/// Doubles the gene when it runs before the `add`.
fn swap(def: &str) -> String {
    let (path, gene, base) = (genes(def), added(def), base_gene(def));
    format!(
        "<Operation Class=\"PatchOperationConditional\">\
         <xpath>{path}/li[text()=\"{gene}\"]</xpath>\
         <match Class=\"PatchOperationRemove\"><xpath>{path}/li[text()=\"{base}\"]</xpath></match>\
         <nomatch Class=\"PatchOperationReplace\"><xpath>{path}/li[text()=\"{base}\"]</xpath>\
         <value><li>{gene}</li></value></nomatch></Operation>"
    )
}

fn base() -> (String, String) {
    (
        xeno(X, &base_gene(X)) + &xeno(Y, &base_gene(Y)),
        String::new(),
    )
}

fn patch(xml: String) -> (String, String) {
    (String::new(), xml)
}

fn names(count: usize) -> Vec<String> {
    ["Base", "C", "I", "D", "E", "F"][..count]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn decided(mods: &[(String, String)], order: &[usize], rules: &[Rule]) -> Found {
    let (_tmp, loaded) = loaded(mods);
    let count = mods.len();
    let resort = |hints: &[Rule]| sort::sort(count, &[rules, hints].concat());
    decide(&loaded, order, rules, &resort, &names(count))
}

fn plain(text: &str) -> Sentence {
    Sentence::Plain(text.into())
}

fn is_part(why: &Sentence, text: &str) -> bool {
    matches!(why, Sentence::Said { parts, .. } if parts.contains(&plain(text)))
}

#[test]
fn the_reported_pair_gives_one_hint() {
    let mods = [base(), patch(swap(X)), patch(add(X))];
    let found = decided(&mods, &[0, 1, 2], &[]);
    assert_eq!(found.unfixed, []);
    let [hint] = &found.hints[..] else {
        panic!("{:?}", found.hints)
    };
    assert_eq!((hint.first, hint.then, hint.code), (2, 1, false));
    for part in ["AG_HeatImmunity", X, "I"] {
        assert!(is_part(&hint.why, part), "{part}: {}", hint.why);
    }
}

#[test]
fn a_pair_touching_one_def_in_both_orders_gives_nothing() {
    let other = "<Operation Class=\"PatchOperationAdd\"><xpath>".to_string()
        + &genes(X)
        + "</xpath><value><li>Other</li></value></Operation>";
    let mods = [base(), patch(other), patch(add(X))];
    assert_eq!(decided(&mods, &[0, 1, 2], &[]), Found::default());
}

#[test]
fn an_error_no_order_fixes_is_unfixed_once_per_file() {
    let lost = "<Operation Class=\"PatchOperationAdd\"><xpath>Defs/XenotypeDef[defName=\"Nobody\"]/genes</xpath>\
                <value><li>G</li></value></Operation>";
    let mods = [base(), patch(lost.repeat(3))];
    let found = decided(&mods, &[0, 1], &[]);
    assert_eq!(found.hints, []);
    let [one] = &found.unfixed[..] else {
        panic!("{:?}", found.unfixed)
    };
    assert_eq!((one.mod_at, one.count), (1, 3));
    assert!(one.file.ends_with("p.xml"));
    assert!(
        one.words.to_string().starts_with("3 errors"),
        "{}",
        one.words
    );
}

#[test]
fn a_rule_pinning_the_bad_order_is_not_swapped() {
    let mods = [base(), patch(swap(X)), patch(add(X))];
    let pin = Rule {
        first: 1,
        then: 2,
        kind: Kind::LoadBefore,
    };
    let found = decided(&mods, &[0, 1, 2], &[pin]);
    assert_eq!(found.hints, []);
    assert_eq!(found.unfixed.len(), 1);
}

#[test]
fn a_hint_dropped_in_a_loop_is_unfixed() {
    let mods = [
        base(),
        patch(swap(Y)),
        patch(swap(X) + &add(Y)),
        patch(add(X)),
    ];
    let pin = Rule {
        first: 1,
        then: 3,
        kind: Kind::Yours,
    };
    let found = decided(&mods, &[0, 1, 2, 3], &[pin]);
    assert_eq!(found.hints.len(), 1, "{:?}", found.hints);
    assert_eq!(found.unfixed.len(), 1, "{:?}", found.unfixed);
}

#[test]
fn an_unknown_def_gives_a_hint_but_no_problem() {
    let custom = "<Operation Class=\"Mod.Custom\"><xpath>Defs/XenotypeDef</xpath></Operation>";
    let lost = "<Operation Class=\"PatchOperationAdd\"><xpath>Defs/XenotypeDef[defName=\"Nobody\"]/genes</xpath>\
                <value><li>G</li></value></Operation>";
    let mods = [base(), patch(swap(X) + custom + lost), patch(add(X))];
    let found = decided(&mods, &[0, 1, 2], &[]);
    assert_eq!(found.hints.len(), 1, "{:?}", found.hints);
    assert_eq!(found.unfixed, []);
}

#[test]
fn a_swap_that_breaks_another_def_is_dropped() {
    let mods = [base(), patch(swap(X) + &add(Y)), patch(add(X) + &swap(Y))];
    let found = decided(&mods, &[0, 1, 2], &[]);
    assert_eq!(found.hints, []);
    assert_eq!(found.unfixed.len(), 1, "{:?}", found.unfixed);
}

#[test]
fn a_failure_that_only_moves_in_the_replay_is_not_a_hint() {
    let never = format!(
        "<li Class=\"PatchOperationReplace\"><xpath>{}/li[text()=\"NeverThere\"]</xpath>\
         <value><li>G</li></value></li>",
        genes(X)
    );
    let first = add(Y)
        .replace(
            "Operation Class=\"PatchOperationAdd\"",
            "li Class=\"PatchOperationAdd\"",
        )
        .replace("</Operation>", "</li>");
    let sequence = format!(
        "<Operation Class=\"PatchOperationSequence\"><operations>{first}{never}</operations></Operation>"
    );
    let mods = [base(), patch(sequence), patch(add(X))];
    let found = decided(&mods, &[0, 1, 2], &[]);
    assert_eq!(found.hints, []);
    assert_eq!(found.unfixed.len(), 1, "{:?}", found.unfixed);
}

#[test]
fn a_swap_keeps_the_errors_it_fixes() {
    let never = format!(
        "<li Class=\"PatchOperationReplace\"><xpath>{}/li[text()=\"NeverThere\"]</xpath>\
         <value><li>G</li></value></li>",
        genes(X)
    );
    let first = add(Y)
        .replace(
            "Operation Class=\"PatchOperationAdd\"",
            "li Class=\"PatchOperationAdd\"",
        )
        .replace("</Operation>", "</li>");
    let sequence = format!(
        "<Operation Class=\"PatchOperationSequence\"><operations>{first}{never}</operations></Operation>"
    );
    let mods = [base(), patch(swap(X) + &sequence), patch(add(X))];
    let found = decided(&mods, &[0, 1, 2], &[]);
    assert_eq!(found.hints.len(), 1, "{:?}", found.hints);
    assert_eq!(found.unfixed.len(), 1, "{:?}", found.unfixed);
    assert_eq!(found.unfixed[0].count, 1);
}

fn against_a_hint(kind: Kind) -> Found {
    let mods = [base(), patch(swap(X)), patch(add(X))];
    let rule = Rule {
        first: 1,
        then: 2,
        kind,
    };
    decided(&mods, &[0, 1, 2], &[rule])
}

#[test]
fn a_simulation_hint_beats_a_file_hint() {
    let found = against_a_hint(Kind::Inferred);
    assert_eq!(found.unfixed, []);
    assert_eq!(found.hints.len(), 1, "{:?}", found.hints);
}

#[test]
fn a_code_hint_beats_a_simulation_hint() {
    let found = against_a_hint(Kind::Code);
    assert_eq!(found.hints, []);
    assert_eq!(found.unfixed.len(), 1, "{:?}", found.unfixed);
}

fn never_there() -> String {
    format!(
        "<Operation Class=\"PatchOperationReplace\"><xpath>{}/li[text()=\"NeverThere\"]</xpath>\
         <value><li>G</li></value></Operation>",
        genes(X)
    )
}

fn look(def: &str) -> String {
    format!(
        "<Operation Class=\"PatchOperationTest\"><xpath>{}</xpath></Operation>",
        genes(def)
    )
}

fn add_value(def: &str, value: &str) -> String {
    format!(
        "<Operation Class=\"PatchOperationAdd\"><xpath>{}</xpath>\
         <value><li>{value}</li></value></Operation>",
        genes(def)
    )
}

fn replays_of(mods: &[(String, String)]) -> u64 {
    let (_tmp, loaded) = loaded(mods);
    let order: Vec<usize> = (0..mods.len()).collect();
    let resort = |hints: &[Rule]| sort::sort(mods.len(), hints);
    decide(&loaded, &order, &[], &resort, &names(mods.len()));
    loaded.replays()
}

#[test]
fn a_pair_after_the_failing_mod_is_not_replayed() {
    let mods = [
        base(),
        patch(never_there()),
        patch(add_value(X, "P")),
        patch(add_value(X, "Q")),
    ];
    assert_eq!(replays_of(&mods), 2);
}

#[test]
fn a_pair_before_the_failing_mod_is_replayed_only_when_it_interacts() {
    let writer = patch(add_value(X, "W"));
    let quiet = [
        base(),
        patch(look(X)),
        patch(look(X)),
        patch(never_there()),
        writer.clone(),
    ];
    assert_eq!(replays_of(&quiet), 5);
    let busy = [
        base(),
        patch(add_value(X, "P")),
        patch(add_value(X, "Q")),
        patch(never_there()),
        writer,
    ];
    assert_eq!(replays_of(&busy), 6);
}
