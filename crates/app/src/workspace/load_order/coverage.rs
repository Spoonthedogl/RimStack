//! `cargo test -p rimstack-app --release patch_coverage_of_the_workshop -- --ignored --nocapture`
//! reads every Workshop mod's patches as if all were active and reports how much of them the simulation
//! reads exactly. Nothing is written to the Workshop or the game.
use super::timing::scratch_app;
use rimstack_core::rimworld::patching::types::Types;
use rimstack_core::rimworld::patching::{self, Active, Loaded as Patching, ModFiles};
use rimstack_core::rimworld::patchsettings::Settings;
use rimstack_core::rimworld::paths::Source;
use rimstack_core::rimworld::{code, game::Game, loadfolders};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

type Who = BTreeSet<usize>;

fn shorten(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(140) {
        Some((at, _)) => format!("{}...", &flat[..at]),
        None => flat,
    }
}

fn digitless(text: &str) -> String {
    let first = text.lines().next().unwrap_or_default();
    first.chars().filter(|c| !c.is_ascii_digit()).collect()
}

/// The first `[...]` of `text`, which starts at its `[`.
fn first_predicate(text: &str) -> Option<&str> {
    let inside = text.strip_prefix('[')?;
    let (mut depth, mut quote) = (0, None);
    for (at, c) in inside.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '[') => depth += 1,
            (None, ']') if depth == 0 => return Some(&inside[..at]),
            (None, ']') => depth -= 1,
            _ => {}
        }
    }
    None
}

fn predicate_shape(rest: &str) -> &'static str {
    let rest = rest.trim_start();
    let Some(p) = first_predicate(rest) else {
        return if rest.is_empty() || !rest.starts_with('[') {
            "no predicate on the def"
        } else {
            "predicate never closes"
        };
    };
    let p = p.trim();
    let digits = !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
    if p.contains("starts-with(") {
        "starts-with("
    } else if p.contains("contains(") {
        "contains("
    } else if p.contains("not(") {
        "not("
    } else if p.contains("last()") || p.contains("position()") || digits {
        "positional"
    } else if p.contains("!=") {
        "!= test"
    } else if p.starts_with('(') {
        "parenthesised names"
    } else if p.contains("@ParentName") {
        "@ParentName test"
    } else if p.contains(" and ") {
        "name test with and"
    } else if p.contains("@Name") && p.contains("defName") {
        "@Name mixed with defName"
    } else if p.contains("@Name") {
        "@Name tests (several, kept narrow only when alone)"
    } else if p
        .strip_prefix("defName")
        .is_some_and(|r| r.trim_start().starts_with('='))
    {
        "plain defName test"
    } else if p.starts_with("defName") || p.starts_with("DefName") {
        "defName test not understood (unparsed)"
    } else if p.starts_with('@') {
        "predicate on another attribute"
    } else if p.starts_with(|c: char| c.is_alphabetic()) {
        "predicate on a field (thingClass=, li, ...)"
    } else {
        "other predicate (unparsed)"
    }
}

/// Why an XPath that is not exact is not, by the first shape found in it.
fn reason(xpath: &str) -> String {
    let path = xpath.trim();
    if path.contains('|') {
        return "| (union)".into();
    }
    if path.contains("//") {
        return "// (descendant)".into();
    }
    if ["parent::", "ancestor", "following", "preceding", ".."]
        .iter()
        .any(|axis| path.contains(axis))
    {
        return "a step out of the def".into();
    }
    if path.trim_matches('/') == "Defs" {
        return "/Defs alone".into();
    }
    let below = path.trim_start_matches('/');
    let (shape, head) = match (below.strip_prefix("Defs/"), below.strip_prefix("*/")) {
        (Some(head), _) => ("", head),
        (None, Some(head)) => ("*/Kind root wildcard; ", head),
        _ => return "does not start with Defs/ or */ (relative, or another root)".into(),
    };
    let end = head.find(['[', '/']).unwrap_or(head.len());
    let (kind, rest) = (head[..end].trim_end(), &head[end..]);
    if kind.contains('*') {
        return format!("{shape}* as the def kind; {}", predicate_shape(rest));
    }
    if kind.contains(['(', '@', ':']) || kind.contains(char::is_whitespace) {
        return format!("{shape}def kind not a plain name");
    }
    format!("{shape}{}", predicate_shape(rest))
}

#[derive(Default)]
struct Group {
    uses: usize,
    mods: Who,
    examples: Vec<String>,
}

impl Group {
    fn add(&mut self, mod_at: usize, example: String, keep: usize) {
        self.uses += 1;
        self.mods.insert(mod_at);
        if self.examples.len() < keep && !self.examples.contains(&example) {
            self.examples.push(example);
        }
    }
}

fn print_groups(title: &str, groups: BTreeMap<String, Group>) {
    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.1.uses));
    let total: usize = sorted.iter().map(|g| g.1.uses).sum();
    eprintln!("{title}: {total} xpaths");
    for (why, group) in sorted {
        eprintln!(
            "  {:>6} xpaths {:>5} mods  {why}",
            group.uses,
            group.mods.len()
        );
        for example in group.examples {
            eprintln!("          e.g. {example}");
        }
    }
}

#[test]
#[ignore = "reads every Workshop mod; run by name"]
fn patch_coverage_of_the_workshop() {
    let (_tmp, app) = scratch_app();
    let loaded = app.library();
    let Ok((library, _)) = &*loaded else {
        panic!("the mods could not be read")
    };
    let mut mods: Vec<_> = library
        .ids()
        .map(|(_, i)| i)
        .filter(|i| i.source == Source::Workshop)
        .collect();
    mods.sort_by(|a, b| a.folder.cmp(&b.folder));
    let ids: Vec<String> = mods.iter().map(|m| m.folder.clone()).collect();
    eprintln!(
        "workshop mods: {}, game version {}",
        mods.len(),
        library.game_version
    );
    let active = Active {
        ids: mods.iter().filter_map(|m| m.key()).collect(),
        names: mods
            .iter()
            .filter_map(|m| m.about.as_ref().ok()?.name.clone())
            .collect(),
    };
    let all = |_: &str| true;
    let files = || -> Vec<ModFiles> {
        mods.iter()
            .map(|m| ModFiles {
                folders: loadfolders::folders(&m.path, &library.game_version, &all),
            })
            .collect()
    };
    let at = |mod_at: usize| ids[mod_at].as_str();

    let probe = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(probe.path().join("Patches")).unwrap();
    let bad = "<Patch><Operation Class=\"PatchOperationRemove\"><xpath>Defs/ThingDef[</xpath></Operation></Patch>";
    std::fs::write(probe.path().join("Patches/a.xml"), bad).unwrap();
    let probed = patching::audit(
        &[ModFiles {
            folders: vec![probe.path().to_path_buf()],
        }],
        &active,
        &Settings::default(),
    );
    assert!(probed.xpaths.iter().all(|x| x.refused) && probed.xpaths.len() == 1);

    let config = app.paths().config.clone();
    let owners = mods.iter().map(|m| m.key()).collect();
    let settings = Settings::read(config.as_deref()).owned_by(owners);
    let seen = patching::audit(&files(), &active, &settings);

    eprintln!("\n== 1. patch files ==");
    eprintln!("total {}, unreadable {}", seen.files, seen.unreadable.len());
    let mut why: BTreeMap<String, Vec<(usize, &PathBuf)>> = BTreeMap::new();
    for (mod_at, path, reason) in &seen.unreadable {
        why.entry(digitless(reason))
            .or_default()
            .push((*mod_at, path));
    }
    let mut why: Vec<_> = why.into_iter().collect();
    why.sort_by_key(|a| std::cmp::Reverse(a.1.len()));
    for (reason, files) in why.iter().take(10) {
        eprintln!("  {:>5}  {reason}", files.len());
    }
    for (mod_at, path, reason) in seen.unreadable.iter().take(10) {
        eprintln!(
            "  e.g. {} {} ({})",
            at(*mod_at),
            path.display(),
            shorten(reason)
        );
    }

    eprintln!("\n== 2. operations by class ==");
    let mut classes: BTreeMap<(&str, bool), (usize, Who)> = BTreeMap::new();
    for (mod_at, class, known) in &seen.operations {
        let entry = classes.entry((class, *known)).or_default();
        entry.0 += 1;
        entry.1.insert(*mod_at);
    }
    let count = |known: bool| -> usize {
        classes
            .iter()
            .filter(|((_, k), _)| *k == known)
            .map(|(_, v)| v.0)
            .sum()
    };
    let (known, unknown) = (count(true), count(false));
    let mods_with_unknown: Who = classes
        .iter()
        .filter(|((_, k), _)| !*k)
        .flat_map(|(_, v)| v.1.iter().copied())
        .collect();
    eprintln!(
        "total {}, known {known}, unknown {unknown} (used by {} mods)",
        known + unknown,
        mods_with_unknown.len()
    );
    let mut known_classes: Vec<_> = classes.iter().filter(|((_, k), _)| *k).collect();
    known_classes.sort_by_key(|a| std::cmp::Reverse(a.1.0));
    for ((class, _), (n, who)) in known_classes {
        eprintln!("  known {n:>7} ops {:>5} mods  {class}", who.len());
    }
    let mut unknown_classes: Vec<_> = classes.iter().filter(|((_, k), _)| !*k).collect();
    unknown_classes.sort_by_key(|a| std::cmp::Reverse(a.1.0));
    for ((class, _), (n, who)) in unknown_classes.into_iter().take(40) {
        eprintln!("  unknown {n:>5} ops {:>5} mods  {class}", who.len());
    }

    eprintln!("\n== 3. XPaths as Target::of_xpath reads them ==");
    let (mut exact, mut kind_only, mut widened) = (0, 0, 0);
    let (mut exact_mods, mut kind_mods, mut wide_mods) = (Who::new(), Who::new(), Who::new());
    let mut kind_groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut wide_groups: BTreeMap<String, Group> = BTreeMap::new();
    for x in &seen.xpaths {
        let example = format!("{} {}", at(x.mod_at), shorten(&x.xpath));
        match (x.target.has_kind(), x.target.has_names()) {
            (_, true) => {
                exact += 1;
                exact_mods.insert(x.mod_at);
            }
            (true, false) => {
                kind_only += 1;
                kind_mods.insert(x.mod_at);
                kind_groups
                    .entry(reason(&x.xpath))
                    .or_default()
                    .add(x.mod_at, example, 3);
            }
            (false, false) => {
                widened += 1;
                wide_mods.insert(x.mod_at);
                wide_groups
                    .entry(reason(&x.xpath))
                    .or_default()
                    .add(x.mod_at, example, 3);
            }
        }
    }
    eprintln!(
        "total {}: exact names {exact} ({} mods), kind with any name {kind_only} ({} mods), widened {widened} ({} mods)",
        seen.xpaths.len(),
        exact_mods.len(),
        kind_mods.len(),
        wide_mods.len()
    );
    print_groups("kind with any name", kind_groups);
    print_groups("widened", wide_groups);

    eprintln!("\n== 4. XPaths libxml2 refuses ==");
    let mut refused: BTreeMap<String, Group> = BTreeMap::new();
    for x in seen.xpaths.iter().filter(|x| x.refused) {
        refused.entry(shorten(&x.xpath)).or_default().add(
            x.mod_at,
            format!("{} {}", at(x.mod_at), x.file.display()),
            3,
        );
    }
    let total: usize = refused.values().map(|g| g.uses).sum();
    eprintln!("refused {total} xpaths, {} distinct", refused.len());
    let mut refused: Vec<_> = refused.into_iter().collect();
    refused.sort_by_key(|a| std::cmp::Reverse(a.1.uses));
    for (xpath, group) in refused.iter().take(40) {
        eprintln!(
            "  {:>5} x {:>4} mods  {xpath}",
            group.uses,
            group.mods.len()
        );
        if let Some(e) = group.examples.first() {
            eprintln!("          e.g. {e}");
        }
    }

    eprintln!("\n== 5. one load of all of them ==");
    let game = app.paths().game.as_deref().and_then(Game::assembly_path);
    let own = mods
        .iter()
        .flat_map(|i| code::assembly_files(&i.path, &library.game_version));
    let dlls: Vec<PathBuf> = game.into_iter().chain(own).collect();
    let started = Instant::now();
    let loaded = Patching::read(files(), &active, &settings, Types::read(&dlls));
    eprintln!(
        "Loaded::read: {:?}, defs {}, unknown classes (top level and nested in known ones) {}",
        started.elapsed(),
        loaded.def_count(),
        loaded.unknown_classes().values().sum::<usize>()
    );
    eprintln!("(lines starting 'Patch simulation skips' above are the notes of files skipped)");
}
