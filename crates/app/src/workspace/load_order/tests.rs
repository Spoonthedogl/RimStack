use super::*;
use crate::workspace::tests::installed;

fn roads_made_for_an_older_game(tmp: &tempfile::TempDir) {
    let about = "<ModMetaData><packageId>Ann.Roads</packageId><name>Ann.Roads</name><supportedVersions><li>1.5</li></supportedVersions></ModMetaData>";
    std::fs::write(tmp.path().join("game/Mods/roads/About/About.xml"), about).unwrap();
}

fn roads(order: &LoadOrder, side: Side) -> ModId {
    let list = order.list(side);
    let m = list.mods().iter().find(|m| m.id == "ann.roads").unwrap();
    m.installed.unwrap()
}

#[test]
fn the_two_lists_are_the_load_order_and_the_rest() {
    let (_tmp, app) = installed();
    let order = LoadOrder::read(&app, true, false);
    assert!(order.problem.is_empty() && order.is_installed());
    let ids = |side| -> Vec<String> {
        order
            .list(side)
            .mods()
            .iter()
            .map(|m| m.id.clone())
            .collect()
    };
    assert_eq!(ids(Side::Active), ["ludeon.rimworld"]);
    assert_eq!(ids(Side::Inactive), ["ann.roads"]);
    assert!(order.status[0].to_string().contains('1'));
}

#[test]
fn a_side_term_filters_its_own_list_and_empties_the_other() {
    let (_tmp, app) = installed();
    let mut order = LoadOrder::read(&app, true, false);
    let matching = |order: &LoadOrder| {
        let count = |side| order.list(side).matching();
        (count(Side::Active), count(Side::Inactive))
    };
    order.set_filter("is:inactive");
    assert_eq!(matching(&order), (0, 1));
    order.set_filter("is:active");
    assert_eq!(matching(&order), (1, 0));
    order.set_filter("");
    assert_eq!(matching(&order), (1, 1));
}

#[test]
fn mods_that_could_not_be_read_give_empty_lists_and_the_reason() {
    let (tmp, app) = installed();
    std::fs::remove_dir_all(tmp.path().join("game")).unwrap();
    let order = LoadOrder::read(&app, true, false);
    assert!(!order.problem.is_empty() && !order.is_installed());
    assert!(order.list(Side::Active).mods().is_empty());
}

#[test]
fn a_mod_taken_across_is_one_step_that_undo_takes_back() {
    let (_tmp, app) = installed();
    let mut order = LoadOrder::read(&app, true, false);
    let id = roads(&order, Side::Inactive);
    order.take_all(&app, Side::Inactive, &[id]);
    assert_eq!(app.session().current(), ["ludeon.rimworld", "ann.roads"]);
    assert!(order.list(Side::Inactive).mods().is_empty());
    assert!(app.session().is_dirty());
    assert!(order.undo(&app));
    assert_eq!(app.session().current(), ["ludeon.rimworld"]);
    assert_eq!(order.list(Side::Inactive).mods().len(), 1);
    assert!(order.redo(&app));
    assert_eq!(order.list(Side::Active).mods().len(), 2);
}

#[test]
fn an_error_ignored_is_gone_until_warned_again() {
    let (tmp, app) = installed();
    let about = "<ModMetaData><packageId>Ann.Roads</packageId><name>Ann.Roads</name><modDependencies><li><packageId>gone.lib</packageId></li></modDependencies></ModMetaData>";
    std::fs::write(tmp.path().join("game/Mods/roads/About/About.xml"), about).unwrap();
    let mut order = LoadOrder::read(&app, true, false);
    let id = roads(&order, Side::Inactive);
    order.take(&app, Side::Inactive, 0, None);
    let errors = |order: &LoadOrder| {
        order
            .troubles()
            .iter()
            .flatten()
            .filter(|t| t.error)
            .count()
    };
    assert_eq!(errors(&order), 1);
    let key = order.troubles()[1][0].key.clone();
    app.ignore_problem("Ann.Roads", &key, true);
    order.recheck(&app);
    assert_eq!(errors(&order), 0);
    let details = serde_json::to_value(order.details(&app, id)).unwrap();
    assert_eq!(details["problemsIgnored"], 1);
    app.warn_again(&[id]);
    order.recheck(&app);
    assert_eq!(errors(&order), 1);
}

#[test]
fn a_mod_made_for_another_game_version_offers_to_trust_it_once_active() {
    let (tmp, app) = installed();
    roads_made_for_an_older_game(&tmp);
    let mut order = LoadOrder::read(&app, true, false);
    let id = roads(&order, Side::Inactive);
    let shown = |order: &mut LoadOrder| serde_json::to_value(order.details(&app, id)).unwrap();
    assert_eq!(shown(&mut order)["offerTrust"], false);
    order.take(&app, Side::Inactive, 0, None);
    let details = shown(&mut order);
    assert_eq!(details["offerTrust"], true, "{}", details["problems"]);
    assert_eq!(details["name"], "Ann.Roads");
}

#[test]
fn a_mod_made_for_another_game_version_is_marked_on_either_side_until_trusted() {
    let (tmp, app) = installed();
    roads_made_for_an_older_game(&tmp);
    let mut order = LoadOrder::read(&app, true, false);
    let id = roads(&order, Side::Inactive);
    let marked = |order: &LoadOrder| {
        let rows = order.rows(&app, Side::Inactive);
        rows.iter()
            .any(|r| r.mod_id == id.to_qml() && r.outdated && r.made_for == "1.5")
    };
    assert!(marked(&order));
    app.set_version_ok(&[id], true);
    order.recheck(&app);
    assert!(!marked(&order));
    app.set_version_ok(&[id], false);
    order.recheck(&app);
    order.take(&app, Side::Inactive, 0, None);
    let active = order.rows(&app, Side::Active);
    assert!(
        active
            .iter()
            .any(|r| r.mod_id == id.to_qml() && r.made_for == "1.5")
    );
}

#[test]
fn saving_writes_the_load_order_and_marks_it_saved() {
    let (tmp, app) = installed();
    let mut order = LoadOrder::read(&app, true, false);
    order.take(&app, Side::Inactive, 0, None);
    order.save(&app).unwrap();
    let written = std::fs::read_to_string(tmp.path().join("config/Config/ModsConfig.xml")).unwrap();
    assert!(written.contains("<li>ann.roads</li>"));
    assert!(!app.session().is_dirty());
}

fn written_outside(tmp: &tempfile::TempDir, ids: &str) -> (std::path::PathBuf, String) {
    let file = tmp.path().join("config/Config/ModsConfig.xml");
    let text = format!("<ModsConfigData><activeMods>{ids}</activeMods></ModsConfigData>");
    std::fs::write(&file, &text).unwrap();
    (file, text)
}

#[test]
fn a_file_written_outside_is_not_saved_over_until_its_list_or_ours_is_kept() {
    let (tmp, app) = installed();
    let mut order = LoadOrder::read(&app, true, false);
    order.take(&app, Side::Inactive, 0, None);
    let (file, text) = written_outside(&tmp, "<li>ludeon.rimworld</li><li>other.mod</li>");

    let refused = order.save(&app).unwrap_err();

    assert_eq!(
        refused,
        rimstack_core::rimworld::modsconfig::changed_outside()
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    assert!(app.session().is_dirty());

    order.keep_file(&app, false);
    assert_eq!(app.session().current(), ["ludeon.rimworld", "ann.roads"]);
    order.save(&app).unwrap();
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("ann.roads") && !written.contains("other.mod"));
}

#[test]
fn taking_the_list_of_a_file_written_outside_leaves_nothing_to_save() {
    let (tmp, app) = installed();
    let mut order = LoadOrder::read(&app, true, false);
    order.take(&app, Side::Inactive, 0, None);
    let (file, text) = written_outside(&tmp, "<li>ludeon.rimworld</li><li>other.mod</li>");

    assert_ne!(app.stamps().mods_config, 0);

    order.keep_file(&app, true);

    assert_eq!(app.session().current(), ["ludeon.rimworld", "other.mod"]);
    assert!(!app.session().is_dirty());
    assert_eq!(app.stamps().mods_config, 0);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

#[test]
fn every_row_has_a_key_of_its_own_and_a_mod_row_its_id() {
    let (tmp, app) = installed();
    roads_made_for_an_older_game(&tmp);
    let mut order = LoadOrder::read(&app, true, false);
    order.take(&app, Side::Inactive, 0, None);
    let rows = order.rows(&app, Side::Active);
    let keys: std::collections::HashSet<_> = rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(keys.len(), rows.len());
    let roads = rows
        .iter()
        .find(|r| r.name.to_string() == "Ann.Roads")
        .unwrap();
    assert!(roads.mod_id >= 0 && roads.number == 2 && roads.outdated);
}

#[test]
fn a_sort_of_a_list_in_order_moves_nothing_and_says_so() {
    let (_tmp, app) = installed();
    let mut order = LoadOrder::read(&app, true, false);
    order.preview_sort(&app);
    assert!(!order.previewing());
    assert!(!order.sort_note.is_empty());
    assert_eq!(order.sort_count().map(|count| count.moves(&app)), Some(0));
}

#[test]
fn made_up_lists_have_rows_and_no_problems() {
    let (_tmp, app) = installed();
    let order = LoadOrder::demo(40, 10);
    assert_eq!(order.list(Side::Active).mods().len(), 40);
    assert!(order.rows(&app, Side::Active).len() >= 40);
    assert_eq!(order.errors_and_warnings(), (0, 0));
}

fn nothing_found() -> crate::workspace::Conflicts {
    crate::workspace::Conflicts {
        members: Vec::new(),
        overlaps: rimstack_core::mods::conflicts::overlaps(&[]),
        scanned: Default::default(),
    }
}

#[test]
fn a_scan_overtaken_by_a_change_of_textures_writes_nothing() {
    let (_tmp, app) = installed();
    let generation = app.conflicts_generation();
    app.textures_changed();
    app.keep_files_read(generation, nothing_found());
    assert!(app.conflicts().reading().is_none());
}

#[test]
fn a_scan_nothing_overtook_is_kept() {
    let (_tmp, app) = installed();
    app.keep_files_read(app.conflicts_generation(), nothing_found());
    assert!(app.conflicts().reading().is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn nothing_is_saved_while_the_game_runs() {
    let (tmp, app) = installed();
    let mut order = LoadOrder::read(&app, true, false);
    order.take(&app, Side::Inactive, 0, None);
    let game = tmp.path().join("game/RimWorld");
    std::fs::copy("/bin/sleep", &game).unwrap();
    let mut running = (0..50)
        .find_map(
            |_| match std::process::Command::new(&game).arg("30").spawn() {
                Err(e) if e.raw_os_error() == Some(26) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    None
                }
                other => Some(other.unwrap()),
            },
        )
        .unwrap();
    let file = tmp.path().join("config/Config/ModsConfig.xml");
    let before = std::fs::read_to_string(&file).unwrap();

    let refused = order.save(&app);

    running.kill().unwrap();
    running.wait().unwrap();
    assert_eq!(
        refused.unwrap_err().kind(),
        rimstack_core::base::error::Kind::Refused
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    order.save(&app).unwrap();
}

fn with_a_mod_gone(workshop_names: &str) -> (tempfile::TempDir, App, LoadOrder) {
    let (tmp, app) = installed();
    std::fs::write(
        tmp.path().join("config/Config/ModsConfig.xml"),
        "<ModsConfigData><activeMods><li>gone.mod</li><li>ludeon.rimworld</li></activeMods></ModsConfigData>",
    )
    .unwrap();
    std::fs::create_dir_all(app.data_dir()).unwrap();
    let names = rimstack_core::rimworld::workshopdb::file_in(app.data_dir());
    std::fs::write(names, workshop_names).unwrap();
    let order = LoadOrder::read(&app, true, false);
    assert_eq!(order.list(Side::Active).mods()[0].id, "gone.mod");
    (tmp, app, order)
}

#[test]
fn a_mod_on_the_list_but_not_installed_gives_its_menu_its_id() {
    let (_tmp, app, order) = with_a_mod_gone("");
    let said = serde_json::to_value(order.not_installed(&app, Side::Active, 0)).unwrap();
    assert_eq!(said["packageId"], "gone.mod");
    assert_eq!(said["fixes"][0]["action"], "search");
    assert_eq!(said["resubscribe"], serde_json::json!([]));
    assert!(order.not_installed(&app, Side::Active, 1).is_none());
    assert!(order.not_installed(&app, Side::Inactive, 0).is_none());
}

#[test]
fn a_mod_not_installed_the_workshop_knows_offers_its_page_and_to_subscribe_again() {
    let (_tmp, app, order) = with_a_mod_gone("gone.mod\t123\tGone");
    let said = serde_json::to_value(order.not_installed(&app, Side::Active, 0)).unwrap();
    assert_eq!(said["name"], "Gone");
    assert_eq!(said["fixes"][0]["action"], "subscribe");
    assert_eq!(said["fixes"][0]["target"], "123");
    assert_eq!(said["resubscribe"], serde_json::json!(["123"]));
}

#[test]
fn a_mod_not_installed_taken_off_the_list_is_gone_from_both() {
    let (_tmp, app, mut order) = with_a_mod_gone("");
    order.take(&app, Side::Active, 0, None);
    assert_eq!(app.session().current(), ["ludeon.rimworld"]);
    let inactive: Vec<&str> = order
        .list(Side::Inactive)
        .mods()
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    assert_eq!(inactive, ["ann.roads"]);
    assert!(order.undo(&app));
    assert_eq!(app.session().current(), ["gone.mod", "ludeon.rimworld"]);
}

/// Core with a xenotype, `c` replacing the gene `i` adds: `c` fails when it loads first.
fn reported_pair() -> (tempfile::TempDir, App, Vec<Mod>) {
    let (tmp, app) = installed();
    let root = tmp.path();
    let write = |file: &str, xml: &str| {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, xml).unwrap();
    };
    write(
        "game/Data/Core/Defs/x.xml",
        "<Defs><XenotypeDef><defName>X</defName><genes><li>Base</li></genes></XenotypeDef></Defs>",
    );
    let about = |id: &str| {
        format!("<ModMetaData><packageId>t.{id}</packageId><name>{id}</name></ModMetaData>")
    };
    let genes = "Defs/XenotypeDef[defName=\"X\"]/genes";
    write("game/Mods/c/About/About.xml", &about("c"));
    write("game/Mods/i/About/About.xml", &about("i"));
    write(
        "game/Mods/c/Patches/p.xml",
        &format!(
            "<Patch><Operation Class=\"PatchOperationReplace\"><xpath>{genes}/li[text()=\"Added\"]</xpath>\
             <value><li>Replaced</li></value></Operation></Patch>"
        ),
    );
    write(
        "game/Mods/i/Patches/p.xml",
        &format!(
            "<Patch><Operation Class=\"PatchOperationAdd\"><xpath>{genes}</xpath>\
             <value><li>Added</li></value></Operation></Patch>"
        ),
    );
    write(
        "config/Config/ModsConfig.xml",
        "<ModsConfigData><activeMods><li>ludeon.rimworld</li><li>t.c</li><li>t.i</li></activeMods></ModsConfigData>",
    );
    let order = LoadOrder::read(&app, false, false);
    let mods = order.list(Side::Active).mods().to_vec();
    (tmp, app, mods)
}

fn place_of(mods: &[Mod], id: &str) -> usize {
    mods.iter().position(|m| m.id == id).unwrap()
}

fn sorted(app: &App, mods: &[Mod]) -> Vec<Mod> {
    let loaded = app.library();
    let Ok((library, _)) = &*loaded else { panic!() };
    let (mods, sorted) =
        rules::sorted_by_category(app, &mut rules::Detected::default(), library, mods.to_vec());
    sorted.order.iter().map(|&at| mods[at].clone()).collect()
}

fn a_turn() -> crate::jobs::Turn {
    crate::jobs::Lane::default().next()
}

#[test]
fn simulated_hints_reach_sort() {
    let (_tmp, app, mods) = reported_pair();
    let before = sorted(&app, &mods);
    assert!(place_of(&before, "t.c") < place_of(&before, "t.i"));

    let found = app.simulate_patches(mods.clone(), &a_turn()).unwrap();
    assert_eq!(found.hints.len(), 1);
    assert!(found.unfixed.is_empty());
    *app.patched().writing() = Some(found);

    let after = sorted(&app, &mods);
    assert!(place_of(&after, "t.i") < place_of(&after, "t.c"));
}

#[test]
fn a_result_for_another_list_is_not_used() {
    let (_tmp, app, mods) = reported_pair();
    let found = app.simulate_patches(mods.clone(), &a_turn()).unwrap();
    *app.patched().writing() = Some(found);
    let loaded = app.library();
    let Ok((library, _)) = &*loaded else { panic!() };
    let patches = |mods: &[Mod]| {
        let found = rules::all_rules(&app, &mut rules::Detected::default(), library, mods);
        found
            .iter()
            .filter(|r| r.kind == rimstack_core::mods::rules::Kind::Patches)
            .count()
    };
    assert_eq!(patches(&mods), 1);
    let without_c: Vec<Mod> = mods.iter().filter(|m| m.id != "t.c").cloned().collect();
    assert_eq!(patches(&without_c), 0);
}

#[test]
fn no_game_defs_no_simulation() {
    let (_tmp, app, mods) = reported_pair();
    std::fs::remove_dir_all(_tmp.path().join("game/Data/Core/Defs")).unwrap();
    app.rescan();
    assert!(app.simulate_patches(mods, &a_turn()).is_none());
}

#[test]
fn a_result_made_before_a_rescan_is_not_kept() {
    let (_tmp, app, mods) = reported_pair();
    let generation = app.conflicts_generation();
    let found = app.simulate_patches(mods.clone(), &a_turn());
    let running = app.simulation_started(&mods);
    app.textures_changed();
    app.keep_patched(generation, found, running);
    assert!(app.patched().reading().is_none());
}

#[test]
fn a_running_simulation_is_told_until_its_result_is_kept() {
    let (_tmp, app, mods) = reported_pair();
    assert!(!app.simulation_running());
    let generation = app.conflicts_generation();
    let guard = app.simulation_started(&mods);
    assert!(app.simulation_running());
    let found = app.simulate_patches(mods, &a_turn());
    assert!(app.simulation_running());
    app.keep_patched(generation, found, guard);
    assert!(app.patched().reading().is_some());
    assert!(!app.simulation_running());
}

#[test]
fn a_stale_result_still_ends_the_running_simulation() {
    let (_tmp, app, mods) = reported_pair();
    let generation = app.conflicts_generation();
    let guard = app.simulation_started(&mods);
    app.textures_changed();
    app.keep_patched(generation, None, guard);
    assert!(!app.simulation_running());
}

#[test]
fn a_simulation_is_needed_for_each_new_list() {
    let (_tmp, app, mods) = reported_pair();
    assert!(app.simulation_needed(&mods));
    let _guard = app.simulation_started(&mods);
    assert!(!app.simulation_needed(&mods));
    let without_c: Vec<Mod> = mods.iter().filter(|m| m.id != "t.c").cloned().collect();
    assert!(app.simulation_needed(&without_c));
    let _again = app.simulation_started(&without_c);
    assert!(app.simulation_needed(&mods));
    app.textures_changed();
    assert!(app.simulation_needed(&without_c));
}

#[test]
fn a_rescan_forgets_the_patched_result() {
    let (_tmp, app, mods) = reported_pair();
    *app.patched().writing() = app.simulate_patches(mods, &a_turn());
    app.textures_changed();
    assert!(app.patched().reading().is_none());
}

#[test]
fn sort_waits_for_a_running_simulation_then_sorts_once() {
    let (_tmp, app, mods) = reported_pair();
    let mut order = LoadOrder::read(&app, true, false);
    let generation = app.conflicts_generation();
    let running = app.simulation_started(&mods);
    order.preview_sort(&app);
    assert!(!order.previewing() && order.sort_waiting());
    order.preview_sort(&app);
    assert!(!order.previewing() && order.sort_waiting());
    order.sort_when_patched(&app);
    assert!(!order.previewing() && order.sort_waiting());

    let found = app.simulate_patches(mods, &a_turn());
    app.keep_patched(generation, found, running);
    order.sort_when_patched(&app);
    assert!(order.previewing() && !order.sort_waiting());
    let shown = order.list(Side::Active).mods().to_vec();
    order.sort_when_patched(&app);
    assert_eq!(order.list(Side::Active).mods(), shown);
}

#[test]
fn a_stale_result_still_ends_the_wait() {
    let (_tmp, app, mods) = reported_pair();
    let mut order = LoadOrder::read(&app, true, false);
    let generation = app.conflicts_generation();
    let running = app.simulation_started(&mods);
    order.preview_sort(&app);
    app.textures_changed();
    app.keep_patched(generation, None, running);
    order.sort_when_patched(&app);
    assert!(!order.sort_waiting());
}

#[test]
fn a_list_changed_while_waiting_cancels_the_wait() {
    let (_tmp, app, mods) = reported_pair();
    let mut order = LoadOrder::read(&app, true, false);
    let running = app.simulation_started(&mods);
    order.preview_sort(&app);
    assert!(order.sort_waiting());
    let at = place_of(order.list(Side::Active).mods(), "t.i");
    order.take(&app, Side::Active, at, None);
    assert!(!order.sort_waiting());
    drop(running);
    order.sort_when_patched(&app);
    assert!(!order.previewing());
}

#[test]
fn no_simulation_no_wait() {
    let (_tmp, app, _mods) = reported_pair();
    let mut order = LoadOrder::read(&app, true, false);
    order.preview_sort(&app);
    assert!(!order.sort_waiting());
    assert!(!order.sort_note.is_empty());
}

/// Empties the kept result's hints: only a reuse gives a result without them.
fn emptied_in_the_record(app: &App) {
    let file = app.patches_file();
    let mut record = rimstack_core::kept::patched::Results::read(&file);
    record.results[0].hints.clear();
    record.write(&file).unwrap();
}

fn kept_results(app: &App) -> usize {
    rimstack_core::kept::patched::Results::read(&app.patches_file())
        .results
        .len()
}

fn hints_of(app: &App, mods: &[Mod]) -> usize {
    app.simulate_patches(mods.to_vec(), &a_turn())
        .unwrap()
        .hints
        .len()
}

#[test]
fn a_result_is_kept_and_reused_while_nothing_it_was_made_from_changed() {
    let (_tmp, app, mods) = reported_pair();
    assert_eq!(hints_of(&app, &mods), 1);
    emptied_in_the_record(&app);
    let again = app.simulate_patches(mods, &a_turn()).unwrap();
    assert!(again.hints.is_empty());
}

#[test]
fn a_reused_result_is_the_one_made() {
    let (_tmp, app, mods) = reported_pair();
    let made = app.simulate_patches(mods.clone(), &a_turn()).unwrap();
    let reused = app.simulate_patches(mods, &a_turn()).unwrap();
    assert_eq!(reused.members, made.members);
    assert_eq!(reused.hints, made.hints);
    assert_eq!(reused.unfixed, made.unfixed);
}

#[test]
fn a_patch_file_touched_makes_the_simulation_run_again() {
    let (tmp, app, mods) = reported_pair();
    assert_eq!(hints_of(&app, &mods), 1);
    emptied_in_the_record(&app);
    let patch = std::fs::File::options()
        .write(true)
        .open(tmp.path().join("game/Mods/c/Patches/p.xml"))
        .unwrap();
    patch
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .unwrap();
    assert_eq!(hints_of(&app, &mods), 1);
}

#[test]
fn another_order_makes_the_simulation_run_again() {
    let (_tmp, app, mods) = reported_pair();
    assert_eq!(hints_of(&app, &mods), 1);
    emptied_in_the_record(&app);
    let mut other = mods.clone();
    other.swap(1, 2);
    app.simulate_patches(other, &a_turn());
    assert_eq!(kept_results(&app), 2);
}

#[test]
fn another_rule_makes_the_simulation_run_again() {
    let (_tmp, app, mods) = reported_pair();
    assert_eq!(hints_of(&app, &mods), 1);
    emptied_in_the_record(&app);
    app.user_rules().writing().add("t.i", "t.c");
    app.simulate_patches(mods, &a_turn());
    assert_eq!(kept_results(&app), 2);
}
