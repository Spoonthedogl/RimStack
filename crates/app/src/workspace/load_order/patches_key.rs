use rayon::prelude::*;
use rimstack_core::mods::rules::Rule;
use rimstack_core::rimworld::patching::{ModFiles, files_read};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Raised when `decide` or what the key covers changes meaning, so an old result is not reused.
const FORMAT: u32 = 1;

/// One mod as the simulation sees it: what names it across sessions and where its files are.
pub(super) struct Part<'a> {
    pub identity: &'a str,
    pub package: Option<String>,
    pub name: &'a str,
    pub folders: &'a [PathBuf],
    pub assemblies: &'a [PathBuf],
}

pub(super) struct Made<'a> {
    pub game_version: &'a str,
    pub game_assembly: Option<&'a Path>,
    pub mods: &'a [Part<'a>],
    pub order: &'a [usize],
    pub rules: &'a [Rule],
    /// The frameworks' settings files, which the simulated patches read.
    pub settings: &'a [PathBuf],
}

/// `DefaultHasher` is SipHash with fixed keys, which is the same on every run of one build; the
/// version in the key covers a build whose hasher differs.
pub(super) fn key_of(made: &Made) -> String {
    let mut hasher = DefaultHasher::new();
    (env!("CARGO_PKG_VERSION"), FORMAT, made.game_version).hash(&mut hasher);
    stat(made.game_assembly).hash(&mut hasher);
    for file in made.settings {
        (file, stat(Some(file))).hash(&mut hasher);
    }
    let digests: Vec<u64> = made.mods.par_iter().map(digest).collect();
    digests.hash(&mut hasher);
    let identity = |at: usize| made.mods.get(at).map(|m| m.identity);
    for &at in made.order {
        identity(at).hash(&mut hasher);
    }
    for rule in made.rules {
        (identity(rule.first), identity(rule.then), rule.kind).hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn digest(part: &Part) -> u64 {
    let mut hasher = DefaultHasher::new();
    (&part.identity, &part.package, part.name).hash(&mut hasher);
    let folders = ModFiles {
        folders: part.folders.to_vec(),
    };
    for file in files_read(&folders).iter().chain(part.assemblies) {
        file.hash(&mut hasher);
        stat(Some(file)).hash(&mut hasher);
    }
    hasher.finish()
}

fn stat(file: Option<&Path>) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(file?).ok()?;
    let stamp = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), stamp.as_nanos()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(settings: &[PathBuf]) -> String {
        key_of(&Made {
            game_version: "1.7",
            game_assembly: None,
            mods: &[],
            order: &[],
            rules: &[],
            settings,
        })
    }

    #[test]
    fn a_changed_settings_file_changes_the_key() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("ModSettingsFrameworkMod_Settings.xml");
        let files = [file.clone(), tmp.path().join("Mod_2574315206_XmlMod.xml")];
        let absent = key(&files);
        std::fs::write(&file, "<a/>").unwrap();
        let written = key(&files);
        assert_ne!(absent, written);
        assert_eq!(written, key(&files));
        std::fs::write(&file, "<a>b</a>").unwrap();
        assert_ne!(written, key(&files));
    }
}
