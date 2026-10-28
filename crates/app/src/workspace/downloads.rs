use super::App;
use rimstack_core::base::elapsed::ago;
use rimstack_core::base::error::Error;
use rimstack_core::base::locks::Shared;
use rimstack_core::base::sentence::Sentence;
use rimstack_core::kept::settings::Switch;
use rimstack_core::kept::updates::{self, LastCheck, Remote};
use rimstack_core::mods::library::{Installed, Library, ModId};
use rimstack_core::rimworld::{paths::Source, workshop};
use rimstack_core::say;
use rimstack_core::sources::downloads::{self, Asked, Behind, Item, Replaceable, Repository};
use rimstack_core::sources::redownload::is_workshop_id;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Checkout {
    pub mod_id: ModId,
    pub name: String,
    pub workshop: bool,
    pub checked: bool,
    pub behind: Option<u64>,
    pub problem: Option<Sentence>,
}

impl Checkout {
    fn is_behind(&self) -> bool {
        self.behind.is_some_and(|n| n > 0) || self.problem.is_some() || !self.checked
    }
}

#[derive(Default)]
pub struct Updates {
    pub checked: Option<u64>,
    pub asked: Option<u64>,
    pub answered: usize,
    pub items: Vec<Behind>,
    pub checkouts: Vec<Checkout>,
    pub problem: Option<Sentence>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BehindShown<'a> {
    #[serde(flatten)]
    pub row: &'a Behind,
    pub asked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesShown<'a> {
    pub checked: Option<u64>,
    pub checked_said: Sentence,
    pub problem: Option<&'a Sentence>,
    pub count: usize,
    /// Of `count`, those SteamCMD put in the game's Mods folder.
    pub steamcmd: usize,
    pub active: Vec<BehindShown<'a>>,
    pub inactive: Vec<BehindShown<'a>>,
    pub checkouts: &'a [Checkout],
    pub checkouts_behind: Vec<&'a Checkout>,
}

pub fn updates_shown<'a>(updates: &'a Updates, asked: &HashSet<String>) -> UpdatesShown<'a> {
    let shown = |active: bool| {
        let rows = updates.items.iter().filter(move |row| row.active == active);
        rows.map(|row| BehindShown {
            row,
            asked: asked.contains(&row.id),
        })
        .collect()
    };
    UpdatesShown {
        checked: updates.checked,
        checked_said: match updates.checked {
            Some(at) => say!(
                "Checked {}",
                ago(super::now_secs().saturating_sub(at)).inside()
            ),
            None => say!("Never checked"),
        },
        problem: updates.problem.as_ref(),
        count: updates.items.len(),
        steamcmd: updates.items.iter().filter(|row| row.steamcmd).count(),
        active: shown(true),
        inactive: shown(false),
        checkouts: &updates.checkouts,
        checkouts_behind: updates.checkouts.iter().filter(|c| c.is_behind()).collect(),
    }
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub items: Vec<Asked>,
    pub repositories: Vec<Repository>,
    pub note: Option<Sentence>,
    pub wanted: Vec<String>,
}

impl App {
    fn checked_file(&self) -> PathBuf {
        self.data_dir().join("updates.json")
    }

    pub fn last_check(&self) -> LastCheck {
        LastCheck::read(&self.checked_file())
    }

    fn keep_check(&self, check: &LastCheck) {
        if let Err(problem) = check.write(&self.checked_file()) {
            rimstack_core::problem!("{problem}");
        }
    }

    /// Last time's answer stays when Steam is not reached.
    pub fn check_updates(&self) -> Result<Updates, Error> {
        let loaded = self.library();
        let (library, _) = loaded.as_ref().as_ref().map_err(Clone::clone)?;
        let mut kept = self.last_check();
        let mut ids: Vec<String> = library
            .workshop_mods()
            .into_keys()
            .chain(self.steamcmd_mods(library).into_iter().map(|(id, _, _)| id))
            .map(String::from)
            .collect();
        ids.sort();
        ids.dedup();
        let problem = match workshop::fetch_details(&ids) {
            Ok(now) => {
                kept.checked = Some(super::now_secs());
                kept.asked = Some(ids.len() as u64);
                kept.now = now.iter().map(|d| (d.id.clone(), d.updated)).collect();
                None
            }
            Err(problem) => Some(problem.sentence().clone()),
        };
        kept.checkouts = self.checkouts_asked(library);
        self.keep_check(&kept);
        let mut report = self.updates_of(library, &kept);
        report.problem = problem;
        Ok(report)
    }

    /// `None` if there was none.
    pub fn kept_updates(&self) -> Option<Updates> {
        let kept = self.last_check();
        if kept == LastCheck::default() {
            return None;
        }
        let loaded = self.library();
        let (library, _) = loaded.as_ref().as_ref().ok()?;
        Some(self.updates_of(library, &kept))
    }

    pub fn checkout_updated(&self, folder: &Path) {
        let mut kept = self.last_check();
        let caught_up = Remote {
            behind: Some(0),
            problem: None,
        };
        let folder = folder.to_string_lossy().into_owned();
        kept.checkouts.insert(folder, caught_up);
        self.keep_check(&kept);
    }

    fn updates_of(&self, library: &Library, kept: &LastCheck) -> Updates {
        let mut on_disk = self.steam_record().0.clone();
        let now: HashMap<String, u64> = kept.now.clone().into_iter().collect();
        let active = self.active_now(library);
        let mut items = downloads::items(library, &active);
        // Steam's copy wins where a mod is in both places.
        for (id, mod_id, have) in self.steamcmd_mods(library) {
            let Some(m) = library.get(mod_id).filter(|_| !items.contains_key(id)) else {
                continue;
            };
            let item = Item {
                mod_id,
                name: m.display_name(),
                active: active.contains(&mod_id),
                steamcmd: true,
            };
            items.insert(id, item);
            on_disk.insert(id.to_string(), have);
        }
        let behind = downloads::behind(&items, &on_disk, &now);
        if kept.checked.is_some() {
            *self.behind().writing() = Some(behind.iter().map(|row| row.id.clone()).collect());
        }
        Updates {
            checked: kept.checked,
            asked: kept.asked,
            answered: now.len(),
            items: behind,
            checkouts: self.checkouts_of(library, &kept.checkouts),
            problem: None,
        }
    }

    fn asked_of_git<'a>(&self, library: &'a Library) -> Vec<(ModId, &'a Installed)> {
        let git_for_workshop = self.settings().switch(Switch::GitForWorkshop);
        library
            .ids()
            .filter(|(_, m)| updates::asks_git(m.source, m.git, git_for_workshop))
            .collect()
    }

    fn checkouts_asked(&self, library: &Library) -> BTreeMap<String, Remote> {
        use rayon::prelude::*;
        let asked = self.asked_of_git(library);
        let answers: Vec<(String, Remote)> = asked
            .par_iter()
            .map(|(_, m)| {
                let said = match rimstack_core::sources::git::behind(&m.path) {
                    Ok(count) => Remote {
                        behind: Some(count as u64),
                        problem: None,
                    },
                    // Kept as a sentence, put in words when it is shown.
                    Err(problem) => Remote {
                        behind: None,
                        problem: Some(problem.sentence().to_kept().to_string()),
                    },
                };
                (m.path.to_string_lossy().into_owned(), said)
            })
            .collect();
        answers.into_iter().collect()
    }

    fn checkouts_of(&self, library: &Library, answers: &BTreeMap<String, Remote>) -> Vec<Checkout> {
        let asked = self.asked_of_git(library).into_iter();
        asked
            .map(|(id, m)| {
                let said = answers.get(m.path.to_string_lossy().as_ref());
                Checkout {
                    mod_id: id,
                    name: m.display_name().to_string(),
                    workshop: m.source == Source::Workshop,
                    checked: said.is_some(),
                    behind: said.and_then(|s| s.behind),
                    problem: said.and_then(|s| s.problem.as_deref().map(Sentence::from_kept)),
                }
            })
            .collect()
    }

    pub fn look_up(&self, pasted: &str) -> Result<Queue, Error> {
        let mut ids = workshop::ids_in(pasted);
        let local = self.paths().local_mods();
        let repositories = downloads::repositories(pasted, local.as_deref());
        // Neither a link nor an id: a mod's name, searched for on the Workshop.
        let searched = ids.is_empty() && repositories.is_empty() && !pasted.contains("://");
        if searched {
            ids = workshop::search(pasted.trim())?;
        }
        if ids.is_empty() {
            let note = match (searched, repositories.is_empty()) {
                (true, _) => say!("No Workshop mod matches that name"),
                (false, true) => say!("No Workshop link, mod id or Git link found"),
                (false, false) => Sentence::default(),
            };
            return Ok(Queue {
                repositories,
                note: Some(note),
                ..Queue::default()
            });
        }
        let ids = workshop::expand_collections(&ids)?;
        let answered = workshop::fetch_details(&ids)?;
        let loaded = self.library();
        let (library, _) = loaded.as_ref().as_ref().map_err(Clone::clone)?;
        let mods = library.workshop_mods();
        let game = &library.game_version;
        let items = downloads::asked(&ids, &answered, &|id| mods.contains_key(id), game);
        // Search results are picked one by one, not all at once.
        let wanted = items
            .iter()
            .filter(|item| !searched && item.title.is_some() && !item.installed)
            .map(|item| item.id.clone())
            .collect();
        Ok(Queue {
            items,
            repositories,
            note: None,
            wanted,
        })
    }

    pub fn replaceable(&self) -> Vec<Replaceable> {
        let loaded = self.library();
        let Ok((library, _)) = &*loaded else {
            return Vec::new();
        };
        let active = self.active_now(library);
        let items = downloads::items(library, &active);
        downloads::replaceable(&items, &self.replacements(), &self.replacements_ignored())
    }

    /// With `None`, every one set aside is suggested again.
    pub fn ignore_replacement(&self, id: Option<ModId>) -> Result<(), Error> {
        let mut ignored: Vec<String> = match id {
            Some(_) => self.replacements_ignored().into_iter().collect(),
            None => Vec::new(),
        };
        if let Some(id) = id.and_then(|id| self.workshop_id(id)) {
            ignored.push(id);
        }
        ignored.sort();
        ignored.dedup();
        self.set_setting("replacementsIgnored", &ignored.join(","))?;
        self.publish(crate::changes::Changes::SETTINGS);
        Ok(())
    }

    fn downloaded_file(&self) -> PathBuf {
        self.data_dir().join("downloaded.json")
    }

    /// Seconds since 1970 when SteamCMD last downloaded each, by Workshop id.
    fn downloaded(&self) -> BTreeMap<String, u64> {
        let kept = rimstack_core::base::atomic_file::read_json(&self.downloaded_file());
        kept.and_then(|json| serde_json::from_value(json).ok())
            .unwrap_or_default()
    }

    pub fn note_downloaded(&self, ids: &[String]) {
        let mut kept = self.downloaded();
        let now = super::now_secs();
        kept.extend(ids.iter().map(|id| (id.clone(), now)));
        let text = serde_json::to_string(&kept).unwrap_or_default();
        if let Err(problem) = rimstack_core::base::atomic_file::write(&self.downloaded_file(), text)
        {
            rimstack_core::problem!("{problem}");
        }
    }

    /// The mods SteamCMD put in the game's Mods folder, each in a folder named by its Workshop
    /// id, with when it was downloaded. One downloaded before that was kept is dated by its
    /// About.xml. A Git checkout is left to Git.
    fn steamcmd_mods<'a>(&self, library: &'a Library) -> Vec<(&'a str, ModId, u64)> {
        let kept = self.downloaded();
        let dated = |m: &Installed| -> Option<u64> {
            let file = std::fs::metadata(m.path.join("About").join("About.xml")).ok()?;
            let since = file.modified().ok()?.duration_since(std::time::UNIX_EPOCH);
            Some(since.ok()?.as_secs())
        };
        library
            .ids()
            .filter(|(_, m)| m.source == Source::Local && !m.git && is_workshop_id(&m.folder))
            .filter_map(|(id, m)| {
                let have = kept.get(&m.folder).copied().or_else(|| dated(m))?;
                Some((m.folder.as_str(), id, have))
            })
            .collect()
    }

    /// The Workshop id of a mod SteamCMD put in the game's Mods folder.
    pub fn steamcmd_id(&self, id: ModId) -> Option<String> {
        let loaded = self.library();
        let (library, _) = loaded.as_ref().as_ref().ok()?;
        let mods = self.steamcmd_mods(library);
        let found = mods.into_iter().find(|(_, mod_id, _)| *mod_id == id);
        found.map(|(folder, _, _)| folder.to_string())
    }

    /// A Workshop mod's id: the name of its folder.
    pub fn workshop_id(&self, id: ModId) -> Option<String> {
        let loaded = self.library();
        let (library, _) = loaded.as_ref().as_ref().ok()?;
        let m = library.get(id).filter(|m| m.source == Source::Workshop)?;
        Some(m.folder.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::tests::installed;

    #[test]
    fn a_local_checkout_is_listed_by_its_mod_with_what_its_remote_said_last() {
        let (tmp, app) = installed();
        let roads = tmp.path().join("game/Mods/roads");
        std::fs::create_dir(roads.join(".git")).unwrap();
        app.rescan();
        let loaded = app.library();
        let (library, _) = loaded.as_ref().as_ref().unwrap();
        let id = library.ids().find(|(_, m)| m.folder == "roads").unwrap().0;

        let report = app.updates_of(library, &LastCheck::default());
        assert_eq!(report.checkouts.len(), 1);
        let row = &report.checkouts[0];
        assert_eq!((row.mod_id, row.checked, row.behind), (id, false, None));
        let shown = updates_shown(&report, &HashSet::new());
        assert_eq!(shown.checkouts_behind.len(), 1, "not asked yet");

        let mut kept = LastCheck::default();
        let remote = Remote {
            behind: Some(2),
            problem: None,
        };
        kept.checkouts
            .insert(roads.to_string_lossy().into_owned(), remote);
        let report = app.updates_of(library, &kept);
        let row = &report.checkouts[0];
        assert_eq!(
            (row.checked, row.behind, row.workshop),
            (true, Some(2), false)
        );
        assert_eq!(crate::qml::values::as_shown(&row)["modId"], id.to_qml());
        drop(loaded);

        app.checkout_updated(&roads);
        let caught_up = app.kept_updates().unwrap();
        assert_eq!(caught_up.checkouts[0].behind, Some(0));
        assert!(
            updates_shown(&caught_up, &HashSet::new())
                .checkouts_behind
                .is_empty()
        );
    }

    #[test]
    fn a_local_mod_has_no_workshop_id_and_ignoring_none_clears_the_list() {
        let (_tmp, app) = installed();
        let loaded = app.library();
        let (library, _) = loaded.as_ref().as_ref().unwrap();
        let roads = library.ids().find(|(_, m)| m.folder == "roads").unwrap().0;
        drop(loaded);
        assert_eq!(app.workshop_id(roads), None);
        app.set_setting("replacementsIgnored", "1,2").unwrap();
        app.ignore_replacement(Some(roads)).unwrap();
        assert_eq!(app.replacements_ignored().len(), 2);
        app.ignore_replacement(None).unwrap();
        assert!(app.replacements_ignored().is_empty());
    }

    #[test]
    fn a_mod_steamcmd_downloaded_is_checked_but_a_checkout_named_by_an_id_is_not() {
        let (tmp, app) = installed();
        let add = |folder: &str, id: &str| {
            let about = tmp.path().join("game/Mods").join(folder).join("About");
            std::fs::create_dir_all(&about).unwrap();
            let xml =
                format!("<ModMetaData><packageId>{id}</packageId><name>{id}</name></ModMetaData>");
            std::fs::write(about.join("About.xml"), xml).unwrap();
        };
        add("2009463077", "Ann.Hats");
        add("818773962", "Ann.Boots");
        std::fs::create_dir(tmp.path().join("game/Mods/818773962/.git")).unwrap();
        app.rescan();
        app.note_downloaded(&["2009463077".to_string()]);
        let loaded = app.library();
        let (library, _) = loaded.as_ref().as_ref().unwrap();
        let find = |folder: &str| library.ids().find(|(_, m)| m.folder == folder).unwrap().0;
        let (hats, boots) = (find("2009463077"), find("818773962"));

        let mut kept = LastCheck {
            checked: Some(1),
            ..LastCheck::default()
        };
        let later = crate::workspace::now_secs() + 60;
        kept.now.insert("2009463077".into(), later);
        kept.now.insert("818773962".into(), later);
        let report = app.updates_of(library, &kept);
        assert_eq!(report.items.len(), 1);
        assert_eq!(
            (report.items[0].mod_id, report.items[0].steamcmd),
            (hats, true)
        );
        kept.now.insert("2009463077".into(), 5);
        assert!(
            app.updates_of(library, &kept).items.is_empty(),
            "downloaded since"
        );
        drop(loaded);
        assert_eq!(app.steamcmd_id(hats).as_deref(), Some("2009463077"));
        assert_eq!(app.steamcmd_id(boots), None);
    }
}
