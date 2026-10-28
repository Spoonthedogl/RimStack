//! Renaming a field here renames what QML must read; each type's test lists its fields.

use serde::Serialize;

/// What was asked, done, or why not: `{ …what came of it }` or `{ problem }`.
#[derive(Serialize)]
#[serde(untagged)]
pub enum Answer<T> {
    Done(T),
    Failed {
        problem: rimstack_core::base::sentence::Sentence,
    },
}

impl<T> From<Result<T, rimstack_core::base::error::Error>> for Answer<T> {
    fn from(outcome: Result<T, rimstack_core::base::error::Error>) -> Self {
        match outcome {
            Ok(done) => Answer::Done(done),
            Err(problem) => Answer::Failed {
                problem: problem.sentence().clone(),
            },
        }
    }
}

/// What a folder typed by hand would be: `ok` when it can be used.
#[derive(Serialize, Clone)]
pub struct FolderVerdict {
    pub ok: bool,
    pub said: rimstack_core::base::sentence::Sentence,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingsReport {
    pub game: Option<String>,
    pub no_mod_folders: bool,
    pub config: Option<String>,
    pub workshop: Option<String>,
    pub game_version: Option<String>,
    pub mods_config: Option<String>,
    pub backup: Option<String>,
    pub set_game: Option<String>,
    pub set_config: Option<String>,
    pub set_workshop: Option<String>,
    pub launch_arguments: Option<String>,
    pub launch_directly: bool,
    pub language: Option<String>,
    pub language_in_use: String,
    pub english: bool,
    pub steamcmd: bool,
    pub interface_scale: String,
    pub font: Option<String>,
    pub community_rules: bool,
    pub version_warnings: bool,
    pub version_aside_for: Option<String>,
    pub file_hints: bool,
    pub git_for_workshop: bool,
    pub git: Option<String>,
    pub self_check: bool,
    pub updates_at_start: bool,
    pub nightly_updates: bool,
    pub logs_kept: usize,
    pub logs_kept_now: usize,
    pub replacements_ignored: Vec<String>,
    pub setup_done: bool,
    pub rimsort_rules: bool,
    pub official: Vec<rimstack_core::rimworld::official::Official>,
    pub data_folder: String,
    pub local_mods: Option<String>,
    pub config_folder: Option<String>,
    pub game_log: Option<String>,
    pub steamcmd_folder: String,
    pub settings_file: String,
    pub diagnostics_file: String,
    pub version: String,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub saves: usize,
    pub folders: usize,
    pub snapshots: usize,
    pub replacements: usize,
    pub rules: usize,
    pub last_launch: Option<LastLaunch>,
}

#[derive(Serialize, Default)]
pub struct LastLaunch {
    pub time: u64,
    pub errors: Option<usize>,
    pub mods: usize,
}

#[derive(Serialize)]
pub struct Written {
    pub file: String,
}

#[derive(Serialize, Default)]
pub struct ShortContrast {
    pub what: &'static str,
    pub found: f64,
    pub needed: f64,
}

/// `{}` when there is none.
#[derive(Serialize)]
#[serde(untagged)]
pub enum ReleaseSaid {
    Newer {
        version: String,
        page: String,
    },
    Latest {},
    Failed {
        problem: rimstack_core::base::sentence::Sentence,
    },
}

#[derive(Serialize, Default)]
pub struct Database {
    pub id: &'static str,
    pub name: &'static str,
    #[serde(rename = "use")]
    pub used_for: rimstack_core::base::sentence::Sentence,
    pub updated: u64,
    pub on: bool,
    pub waiting: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_its_result_or_its_problem_and_never_both() {
        let failed: Answer<Written> = Err(rimstack_core::base::error::Error::refused("no")).into();
        assert_eq!(
            crate::qml::values::as_shown(&failed),
            serde_json::json!({ "problem": "no" })
        );
        let done: Answer<Written> = Ok(Written { file: "/a".into() }).into();
        assert_eq!(
            crate::qml::values::as_shown(&done),
            serde_json::json!({ "file": "/a" })
        );
    }
}
