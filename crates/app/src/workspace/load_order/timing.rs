//! `cargo test -p rimstack-app --release patch_simulation_timing_on_this_install -- --ignored --nocapture`
//! reads the installed game and mods; settings and data go to scratch folders and the active list is
//! copied, so nothing of the game's is written.
use super::rules::Replay;
use super::*;
use rimstack_core::mods::patch_order;
use rimstack_core::rimworld::patching::types::Types;
use rimstack_core::rimworld::patching::{self, Active, Failure, Loaded as Patching, ModFiles};
use rimstack_core::rimworld::paths::Overrides;
use rimstack_core::rimworld::{code, game::Game, loadfolders};
use std::path::PathBuf;
use std::time::Instant;

/// An `App` on scratch settings and data, with the real active list copied in.
pub(super) fn scratch_app() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut given = Overrides::from_env();
    let real = std::env::var_os("RIMSTACK_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config/unity3d/Ludeon Studios/RimWorld by Ludeon Studios"));
    std::fs::create_dir_all(root.join("config/Config")).unwrap();
    std::fs::copy(
        real.join("Config/ModsConfig.xml"),
        root.join("config/Config/ModsConfig.xml"),
    )
    .unwrap();
    given.config = Some(root.join("config"));
    let rules = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("RimStack/communityRules.json");
    std::fs::create_dir_all(root.join("data/RimStack")).unwrap();
    let _ = std::fs::copy(rules, root.join("data/RimStack/communityRules.json"));
    let app = App::new(home, &root.join("settings"), &root.join("data"), given);
    (tmp, app)
}

#[test]
#[ignore = "reads the installed game and mods; run by name"]
fn patch_simulation_timing_on_this_install() {
    let (_tmp, app) = scratch_app();
    let order = LoadOrder::read(&app, true, false);
    if let Some(members) = order.files_to_read(&app) {
        app.read_active_files(members);
    }
    let order = LoadOrder::read(&app, true, false);
    let mods = order.list(Side::Active).mods().to_vec();
    let loaded = app.library();
    let Ok((library, _)) = &*loaded else {
        panic!("the mods could not be read")
    };
    let installed = |m: &Mod| m.installed.and_then(|i| library.get(i));
    let active = Active {
        ids: mods.iter().filter_map(|m| installed(m)?.key()).collect(),
        names: mods
            .iter()
            .filter_map(|m| installed(m)?.about.as_ref().ok()?.name.clone())
            .collect(),
    };
    let folders: Vec<Vec<PathBuf>> = mods
        .iter()
        .map(|m| {
            let is_active = |k: &str| active.ids.contains(k);
            installed(m)
                .map(|i| loadfolders::folders(&i.path, &library.game_version, &is_active))
                .unwrap_or_default()
        })
        .collect();
    let replay = Replay::new(&app, library, &mods);
    let order = replay.sort(&[]).order;
    let rules = replay.rules();
    let names: Vec<String> = mods.iter().map(|m| m.name.clone()).collect();
    let game = app.paths().game.as_deref().and_then(Game::assembly_path);
    let own = mods
        .iter()
        .filter_map(installed)
        .flat_map(|i| code::assembly_files(&i.path, &library.game_version));
    let dlls: Vec<PathBuf> = game.into_iter().chain(own).collect();

    let started = Instant::now();
    let files = folders.into_iter().map(|folders| ModFiles { folders });
    let patching = Patching::read(files.collect(), &active, Types::read(&dlls));
    eprintln!(
        "active mods: {}, defs: {}",
        mods.len(),
        patching.def_count()
    );
    eprintln!("Loaded::read: {:?}", started.elapsed());

    let started = Instant::now();
    let first = patching.run(&order, None);
    eprintln!("Loaded::run (full): {:?}", started.elapsed());
    let (mut failed, mut duplicate, mut twice) = (0, 0, 0);
    for f in &first.failures {
        match f {
            Failure::Failed { .. } => failed += 1,
            Failure::Duplicate { .. } => duplicate += 1,
            Failure::Twice { .. } => twice += 1,
        }
    }
    let wide = first.unknown.iter().filter(|t| !t.has_kind()).count();
    eprintln!("unknown targets: {}, widened: {wide}", first.unknown.len());
    let mut shapes = std::collections::BTreeMap::new();
    for t in &first.unknown {
        *shapes
            .entry((t.has_kind(), t.has_names()))
            .or_insert(0usize) += 1;
    }
    eprintln!("unknown target shapes (every side has a kind, names): {shapes:?}");
    for f in &first.failures {
        if let Failure::Failed {
            file,
            operation,
            xpath,
            target,
            ..
        } = f
        {
            eprintln!("failed: {} {operation} {xpath} {target:?}", file.display());
        }
    }
    let mut classes: Vec<_> = patching.unknown_classes().into_iter().collect();
    classes.sort_by_key(|class| std::cmp::Reverse(class.1));
    for (class, n) in classes.iter().take(20) {
        eprintln!("unknown class {n:>5}  {class}");
    }
    eprintln!("first run failures: {failed} failed, {duplicate} duplicate, {twice} twice");

    let (before, spent) = patching::subset_runs();
    let started = Instant::now();
    let resort = |extra: &[_]| replay.sort(extra);
    let found = patch_order::decide(&patching, &order, &rules, &resort, &names);
    let overall = started.elapsed();
    let (after, in_subsets) = patching::subset_runs();
    eprintln!(
        "decide: {overall:?} overall (includes its own full run); {} subset replays, {in_subsets:?} in them (before decide: {before}, {spent:?})",
        after - before
    );
    eprintln!(
        "hints {}, unfixed {}",
        found.hints.len(),
        found.unfixed.len()
    );
    eprintln!("hints: {:#?}", found.hints);
    eprintln!("unfixed: {:#?}", found.unfixed);
}

#[test]
#[ignore = "reads the installed game and mods; run by name"]
fn patch_result_key_and_reuse_on_this_install() {
    use super::patches_key::{Made, Part, key_of};
    let (_tmp, app) = scratch_app();
    let order = LoadOrder::read(&app, true, false);
    if let Some(members) = order.files_to_read(&app) {
        app.read_active_files(members);
    }
    let order = LoadOrder::read(&app, true, false);
    let mods = order.list(Side::Active).mods().to_vec();
    let turn = || crate::jobs::Lane::default().next();

    let started = Instant::now();
    let made = app.simulate_patches(mods.clone(), &turn());
    eprintln!("simulated and kept: {:?}", started.elapsed());
    let started = Instant::now();
    let reused = app.simulate_patches(mods.clone(), &turn());
    eprintln!("second run, reused: {:?}", started.elapsed());
    assert_eq!(
        made.map(|p| (p.hints.len(), p.unfixed.len())),
        reused.map(|p| (p.hints.len(), p.unfixed.len()))
    );

    let loaded = app.library();
    let Ok((library, _)) = &*loaded else {
        panic!("the mods could not be read")
    };
    let installed = |m: &Mod| m.installed.and_then(|i| library.get(i));
    let active = Active {
        ids: mods.iter().filter_map(|m| installed(m)?.key()).collect(),
        names: Default::default(),
    };
    let folders: Vec<Vec<PathBuf>> = mods
        .iter()
        .map(|m| {
            let is_active = |k: &str| active.ids.contains(k);
            installed(m)
                .map(|i| loadfolders::folders(&i.path, &library.game_version, &is_active))
                .unwrap_or_default()
        })
        .collect();
    let own: Vec<Vec<PathBuf>> = mods
        .iter()
        .map(|m| {
            installed(m)
                .map(|i| code::assembly_files(&i.path, &library.game_version))
                .unwrap_or_default()
        })
        .collect();
    let identities: Vec<String> = mods
        .iter()
        .map(|m| installed(m).map_or_else(|| m.id.clone(), |i| i.path.to_string_lossy().into()))
        .collect();
    let parts: Vec<Part> = mods
        .iter()
        .zip(&identities)
        .zip(folders.iter().zip(&own))
        .map(|((m, identity), (folders, assemblies))| Part {
            identity,
            package: installed(m).and_then(|i| i.key()),
            name: &m.name,
            folders,
            assemblies,
        })
        .collect();
    let replay = Replay::new(&app, library, &mods);
    let order = replay.sort(&[]).order;
    let rules = replay.rules();
    let game = app.paths().game.as_deref().and_then(Game::assembly_path);
    let started = Instant::now();
    let key = key_of(&Made {
        game_version: &library.game_version,
        game_assembly: game.as_deref(),
        mods: &parts,
        order: &order,
        rules: &rules,
    });
    eprintln!("key {key} alone: {:?}", started.elapsed());
}
