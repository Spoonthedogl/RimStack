use super::App;
use rimstack_core::base::error::{Error, Result};
use rimstack_core::base::sentence::Sentence;
use rimstack_core::mods::library::ModId;
use rimstack_core::sources::{git, redownload};
use rimstack_core::{said, say};
use std::path::PathBuf;

pub struct Done {
    pub note: Result<Sentence>,
    pub wrote: bool,
}

impl Done {
    fn told(note: Result<Sentence>) -> Self {
        Self { note, wrote: false }
    }

    fn of_a_write(note: Result<Sentence>) -> Self {
        Self {
            wrote: note.is_ok(),
            note,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamAsk {
    Subscribe,
    Unsubscribe,
    Download,
}

impl SteamAsk {
    fn word(self) -> &'static str {
        match self {
            SteamAsk::Subscribe => "subscribe",
            SteamAsk::Unsubscribe => "unsubscribe",
            SteamAsk::Download => "download",
        }
    }
}

impl App {
    fn mod_folder(&self, id: ModId) -> Result<(PathBuf, String)> {
        let loaded = self.library();
        let (library, _) = loaded.as_ref().as_ref().map_err(Clone::clone)?;
        let m = library.get(id).ok_or_else(|| {
            Error::refused(said!(
                "That mod is no longer where it was read; read the folders again"
            ))
        })?;
        Ok((m.path.clone(), m.display_name().to_string()))
    }

    /// Through the program started again: Steam shows the game as running
    /// while its library is in use.
    pub fn ask_steam(&self, what: SteamAsk, ids: &[String]) -> Done {
        let request = format!("--steamworks={}:{}", what.word(), ids.join(","));
        // On Linux through /proc: the path of a program rebuilt or
        // updated while it runs no longer leads to it.
        let own = std::path::Path::new("/proc/self/exe");
        let program = match own.exists() {
            true => Ok(own.to_path_buf()),
            false => std::env::current_exe(),
        };
        let ran = program.and_then(|program| {
            rimstack_core::base::program::run(
                rimstack_core::base::program::command(program).arg(&request),
                rimstack_core::base::program::NETWORK,
            )
        });
        Done::told(match ran {
            Ok(out) if out.status.success() && what == SteamAsk::Download => {
                Ok(say!("Steam was asked to update {}", ids.len()))
            }
            Ok(out) if out.status.success() => Ok(say!("Steam was asked about {}", ids.len())),
            Ok(out) => Err(Error::program(said!(
                "Steam did not do all it was asked: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))),
            Err(problem) => Err(Error::program(said!("Steam was not asked: {}", problem))),
        })
    }

    pub fn update_checkout(&self, id: ModId) -> Done {
        let (folder, name) = match self.mod_folder(id) {
            Ok(found) => found,
            Err(problem) => return Done::told(Err(problem)),
        };
        let updated = git::update(&folder);
        if updated.is_ok() {
            self.checkout_updated(&folder);
        }
        Done::of_a_write(match updated {
            Ok(commit) => Ok(say!("Updated {}: {}", name, commit)),
            Err(problem) => Err(Error::new(
                problem.kind(),
                said!("Not updated: {}", problem.within()),
            )),
        })
    }

    pub fn clone_repository(&self, link: &str) -> Done {
        let Some(mods) = self.paths().local_mods() else {
            return Done::told(Err(Error::refused(said!("The game's folder is not known"))));
        };
        Done::of_a_write(match git::clone(link, &mods) {
            Ok(into) => Ok(say!("Cloned {} into {}", link, into.display())),
            Err(problem) => Err(Error::new(
                problem.kind(),
                said!("Not cloned: {}", problem.within()),
            )),
        })
    }

    pub fn install_steamcmd(&self) -> Done {
        Done::told(match redownload::install(self.data_dir()) {
            Ok(_) => {
                self.look_for_steamcmd();
                Ok(say!("SteamCMD installed"))
            }
            Err(problem) => Err(Error::new(
                problem.kind(),
                said!("SteamCMD not installed: {}", problem.within()),
            )),
        })
    }

    pub fn download_with_steamcmd(&self, ids: &[String]) -> Done {
        let steamcmd = (*self.steamcmd()).clone();
        let (steamcmd, local) = match (steamcmd, self.paths().local_mods()) {
            (Some(steamcmd), Some(local)) => (steamcmd, local),
            (None, _) => {
                return Done::told(Err(Error::refused(said!("SteamCMD is not installed"))));
            }
            (_, None) => {
                return Done::told(Err(Error::refused(said!("The game's folder is not known"))));
            }
        };
        let scratch = self.data_dir().join("steamcmd-downloads");
        let fetched = redownload::fetch_all(&steamcmd, ids, &scratch);
        let outcomes: Vec<Result<PathBuf>> = fetched
            .items
            .iter()
            .map(|(id, fresh)| match fresh {
                Ok(fresh) => redownload::put_in_local(fresh, id, &local),
                Err(problem) => Err(problem.clone()),
            })
            .collect();
        let downloaded: Vec<String> = fetched
            .items
            .iter()
            .zip(&outcomes)
            .filter(|(_, outcome)| outcome.is_ok())
            .map(|((id, _), _)| id.clone())
            .collect();
        self.note_downloaded(&downloaded);
        let failed: Vec<&Error> = outcomes.iter().filter_map(|o| o.as_ref().err()).collect();
        Done {
            wrote: failed.len() < outcomes.len(),
            note: match failed.first() {
                None => Ok(say!("{} downloaded", outcomes.len())),
                Some(first) => Err(Error::new(
                    first.kind(),
                    said!(
                        "{} of {} not downloaded: {}",
                        failed.len(),
                        outcomes.len(),
                        first.within()
                    ),
                )),
            },
        }
    }

    /// Slow when a crash left a copy to delete: off the window's thread, before the library is
    /// read.
    pub fn settle_downloads(&self) {
        let Some(local) = self.paths().local_mods() else {
            return;
        };
        for problem in redownload::settle_within(&local) {
            rimstack_core::problem!("{problem}");
        }
    }

    pub fn redownload(&self, id: ModId) -> Done {
        let found = self.mod_folder(id).and_then(|(folder, _)| {
            let workshop = self
                .workshop_id(id)
                .or_else(|| self.steamcmd_id(id))
                .ok_or_else(|| Error::refused(said!("Not a Workshop mod")))?;
            Ok((folder, workshop))
        });
        let scratch = self.data_dir().join("steamcmd");
        let done = found.and_then(|(folder, workshop)| {
            let steamcmd = (*self.steamcmd())
                .clone()
                .ok_or_else(|| Error::refused(said!("SteamCMD is not installed")))?;
            let fetched =
                redownload::fetch_all(&steamcmd, std::slice::from_ref(&workshop), &scratch);
            redownload::replace(&folder, fetched.first()?)?;
            self.note_downloaded(&[workshop]);
            Ok(())
        });
        Done::of_a_write(match done {
            Ok(()) => Ok(say!("Downloaded again")),
            Err(problem) => Err(Error::new(
                problem.kind(),
                said!("Not downloaded again: {}", problem.within()),
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::workspace::tests::installed;
    use rimstack_core::base::error::Kind;
    use rimstack_core::mods::library::ModId;
    use std::fs;

    #[test]
    fn a_local_mod_is_not_downloaded_again_and_a_gone_one_is_not_updated() {
        let (_tmp, app) = installed();
        let loaded = app.library();
        let (library, _) = loaded.as_ref().as_ref().unwrap();
        let roads = library.ids().find(|(_, m)| m.folder == "roads").unwrap().0;
        drop(loaded);
        let done = app.redownload(roads);
        assert_eq!(done.note.unwrap_err().kind(), Kind::Refused);
        assert!(!done.wrote);
        let gone = app.update_checkout(ModId::new(0, 99));
        assert_eq!(gone.note.unwrap_err().kind(), Kind::Refused);
        let not_git = app.update_checkout(roads);
        assert!(not_git.note.is_err() && !not_git.wrote);
    }

    #[test]
    fn a_mod_left_aside_by_a_crash_is_back_when_the_library_is_read() {
        let (tmp, app) = installed();
        let mods = tmp.path().join("game/Mods");
        fs::rename(mods.join("roads"), mods.join("roads.rimstack-old")).unwrap();
        fs::create_dir_all(mods.join("roads.rimstack-new/About")).unwrap();
        app.settle_downloads();
        let loaded = app.library();
        let (library, _) = loaded.as_ref().as_ref().unwrap();
        let folders: Vec<&str> = library.ids().map(|(_, m)| m.folder.as_str()).collect();
        assert!(folders.contains(&"roads"), "{folders:?}");
        assert!(!mods.join("roads.rimstack-old").exists());
        assert!(!mods.join("roads.rimstack-new").exists());
    }
}
