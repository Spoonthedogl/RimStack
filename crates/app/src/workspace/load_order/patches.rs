use super::patches_key::{Made, Part, key_of};
use super::rules::Replay;
use super::*;
use crate::jobs::Turn;
use rimstack_core::base::locks::Held;
use rimstack_core::kept::patched as kept;
use rimstack_core::mods::patch_order::{self, Unfixed};
use rimstack_core::rimworld::patching::types::Types;
use rimstack_core::rimworld::patching::{Active, Loaded as Patching, ModFiles};
use rimstack_core::rimworld::paths::Source;
use rimstack_core::rimworld::{code, game::Game, loadfolders};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Kept by `ModId`, never by place.
pub struct Patched {
    /// Ascending, as `Conflicts`.
    pub members: Vec<ModId>,
    pub hints: Vec<(ModId, ModId, Sentence)>,
    pub unfixed: Vec<(ModId, Unfixed)>,
}

/// Counts as running from where it is made, on the window's thread, to where it drops.
pub struct Simulating(Arc<AtomicUsize>);

impl Drop for Simulating {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl App {
    pub(crate) fn patches_file(&self) -> PathBuf {
        self.data_dir().join("patches.json")
    }

    pub fn simulation_started(&self, mods: &[Mod]) -> Simulating {
        *self.simulated_for().held() = Some(members_of(mods));
        self.simulations().fetch_add(1, Ordering::SeqCst);
        Simulating(self.simulations().clone())
    }

    /// Whether the newest simulation, if any, was not for this very list of mods.
    pub fn simulation_needed(&self, mods: &[Mod]) -> bool {
        self.simulated_for().held().as_deref() != Some(members_of(mods).as_slice())
    }

    pub fn simulation_running(&self) -> bool {
        self.simulations().load(Ordering::SeqCst) > 0
    }

    /// Dropped when the library or the files changed since `generation`; a waiting Sort is told either way.
    /// `running` ends here, once the slot holds the result and before anyone is told.
    pub fn keep_patched(&self, generation: u64, found: Option<Patched>, running: Simulating) {
        let mut slot = self.patched().writing();
        if self.conflicts_generation() == generation {
            *slot = found;
        }
        drop(slot);
        drop(running);
        self.publish(crate::changes::Changes::PATCHES);
    }

    /// A panic inside is logged and gives no result: Sort goes on without hints.
    pub(crate) fn simulate_patches_caught(&self, mods: Vec<Mod>, turn: &Turn) -> Option<Patched> {
        let found = catch_unwind(AssertUnwindSafe(|| self.simulate_patches(mods, turn)));
        found.unwrap_or_else(|_| {
            rimstack_core::problem!("the patch simulation failed; Sort goes on without its hints");
            None
        })
    }

    /// Off the window's thread: the files' nodes cannot leave the one that reads them.
    /// `None` without the game's defs, or once `turn` is not the newest.
    pub(crate) fn simulate_patches(&self, mods: Vec<Mod>, turn: &Turn) -> Option<Patched> {
        let started = std::time::Instant::now();
        let loaded = self.library();
        let Ok((library, _)) = &*loaded else {
            return None;
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
                let folders = installed(m).map(|i| {
                    let is_active = |k: &str| active.ids.contains(k);
                    loadfolders::folders(&i.path, &library.game_version, &is_active)
                });
                folders.unwrap_or_default()
            })
            .collect();
        let game_defs = mods.iter().zip(&folders).any(|(m, folders)| {
            installed(m).is_some_and(|i| i.source == Source::Official)
                && folders.iter().any(|f| f.join("Defs").is_dir())
        });
        if !game_defs || !turn.current() {
            return None;
        }

        let replay = Replay::new(self, library, &mods);
        let order = replay.sort(&[]).order;
        let rules = replay.rules();
        let names: Vec<String> = mods.iter().map(|m| m.name.clone()).collect();
        let game = self.paths().game.as_deref().and_then(Game::assembly_path);
        let own: Vec<Vec<PathBuf>> = mods
            .iter()
            .map(|m| {
                let files =
                    installed(m).map(|i| code::assembly_files(&i.path, &library.game_version));
                files.unwrap_or_default()
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
        let key = key_of(&Made {
            game_version: &library.game_version,
            game_assembly: game.as_deref(),
            mods: &parts,
            order: &order,
            rules: &rules,
        });
        let record = kept::Results::read(&self.patches_file());
        if let Some(result) = record.get(&key) {
            rimstack_core::note!(
                "patch_simulation_reused_ms={}",
                started.elapsed().as_millis()
            );
            let places: Vec<&str> = identities.iter().map(String::as_str).collect();
            return turn.current().then(|| from_kept(result, &mods, &places));
        }
        let dlls: Vec<PathBuf> = game
            .into_iter()
            .chain(own.iter().flatten().cloned())
            .collect();
        let files = folders.iter().map(|folders| ModFiles {
            folders: folders.clone(),
        });
        let patching = Patching::read(files.collect(), &active, Types::read(&dlls));
        if !turn.current() {
            return None;
        }
        let resort = |extra: &[_]| replay.sort(extra);
        let found = patch_order::decide(&patching, &order, &rules, &resort, &names);

        let id = |at: usize| mods.get(at)?.installed;
        let patched = Patched {
            members: members_of(&mods),
            hints: found
                .hints
                .into_iter()
                .filter_map(|h| Some((id(h.first)?, id(h.then)?, h.why)))
                .collect(),
            unfixed: found
                .unfixed
                .into_iter()
                .filter_map(|u| Some((id(u.mod_at)?, u)))
                .collect(),
        };
        if turn.current() {
            let places: Vec<&str> = identities.iter().map(String::as_str).collect();
            let mut record = record;
            record.keep(to_kept(key, &patched, &mods, &places));
            if let Err(problem) = record.write(&self.patches_file()) {
                rimstack_core::note!("the patch simulation's result was not kept: {problem}");
            }
        }
        rimstack_core::note!("patch_simulation_ms={}", started.elapsed().as_millis());
        turn.current().then_some(patched)
    }
}

/// `folders` names each place of `mods`, as the key does.
fn from_kept(result: &kept::Result, mods: &[Mod], folders: &[&str]) -> Patched {
    let place = |folder: &str| folders.iter().position(|f| *f == folder);
    let id = |folder: &str| mods.get(place(folder)?)?.installed;
    Patched {
        members: members_of(mods),
        hints: result
            .hints
            .iter()
            .filter_map(|h| Some((id(&h.first)?, id(&h.then)?, h.why.clone())))
            .collect(),
        unfixed: result
            .unfixed
            .iter()
            .filter_map(|u| {
                let mod_at = place(&u.mod_folder)?;
                let unfixed = Unfixed {
                    mod_at,
                    file: PathBuf::from(&u.file),
                    count: u.count,
                    words: u.words.clone(),
                };
                Some((mods[mod_at].installed?, unfixed))
            })
            .collect(),
    }
}

fn to_kept(key: String, patched: &Patched, mods: &[Mod], folders: &[&str]) -> kept::Result {
    let folder = |id: ModId| {
        let at = mods.iter().position(|m| m.installed == Some(id))?;
        folders.get(at).map(|f| f.to_string())
    };
    kept::Result {
        key,
        hints: patched
            .hints
            .iter()
            .filter_map(|(first, then, why)| {
                Some(kept::Hint {
                    first: folder(*first)?,
                    then: folder(*then)?,
                    why: why.clone(),
                })
            })
            .collect(),
        unfixed: patched
            .unfixed
            .iter()
            .filter_map(|(id, u)| {
                Some(kept::Unfixed {
                    mod_folder: folder(*id)?,
                    file: u.file.to_string_lossy().into_owned(),
                    count: u.count,
                    words: u.words.clone(),
                })
            })
            .collect(),
    }
}

pub(super) fn members_of(mods: &[Mod]) -> Vec<ModId> {
    let mut members: Vec<ModId> = mods.iter().filter_map(|m| m.installed).collect();
    members.sort_unstable();
    members.dedup();
    members
}
