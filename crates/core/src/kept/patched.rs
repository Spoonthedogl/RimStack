use crate::base::sentence::Sentence;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Results kept at most, so a mod toggled off and on finds both.
const MOST: usize = 8;

/// Kept in `patches.json`: the patch simulation's results, newest first. Mods are named by their
/// folder, which outlives a session where a `ModId` does not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Results {
    pub results: Vec<Result>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Result {
    pub key: String,
    pub hints: Vec<Hint>,
    pub unfixed: Vec<Unfixed>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hint {
    pub first: String,
    pub then: String,
    #[serde(with = "kept_sentence")]
    pub why: Sentence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unfixed {
    pub mod_folder: String,
    pub file: String,
    pub count: usize,
    #[serde(with = "kept_sentence")]
    pub words: Sentence,
}

mod kept_sentence {
    use super::Sentence;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(said: &Sentence, to: S) -> Result<S::Ok, S::Error> {
        said.to_kept().serialize(to)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(from: D) -> Result<Sentence, D::Error> {
        Ok(Sentence::from_kept_value(&serde_json::Value::deserialize(
            from,
        )?))
    }
}

impl Results {
    /// A missing or damaged file is nothing kept.
    pub fn read(file: &Path) -> Self {
        let kept = crate::base::atomic_file::read_json(file).map(serde_json::from_value);
        kept.and_then(std::result::Result::ok).unwrap_or_default()
    }

    pub fn write(&self, file: &Path) -> std::result::Result<(), crate::base::error::Error> {
        let text = serde_json::to_string(self)
            .map_err(|e| crate::base::error::Error::format(format!("{}: {e}", file.display())))?;
        crate::base::atomic_file::write(file, text)
    }

    pub fn get(&self, key: &str) -> Option<&Result> {
        self.results.iter().find(|r| r.key == key)
    }

    /// Newest first; the same key is replaced and the oldest beyond `MOST` is dropped.
    pub fn keep(&mut self, result: Result) {
        self.results.retain(|r| r.key != result.key);
        self.results.insert(0, result);
        self.results.truncate(MOST);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(key: &str) -> Result {
        Result {
            key: key.into(),
            hints: vec![Hint {
                first: "/mods/i".into(),
                then: "/mods/c".into(),
                why: Sentence::Said {
                    english: "{} needs {}".into(),
                    parts: vec![Sentence::Plain("i".into()), Sentence::Plain("c".into())],
                },
            }],
            unfixed: vec![Unfixed {
                mod_folder: "/mods/c".into(),
                file: "Patches/p.xml".into(),
                count: 2,
                words: Sentence::Plain("failed".into()),
            }],
        }
    }

    #[test]
    fn results_are_kept_and_read_back_with_their_sentences() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("patches.json");
        assert_eq!(Results::read(&file), Results::default());
        let mut kept = Results::default();
        kept.keep(result("a"));
        kept.write(&file).unwrap();
        let read = Results::read(&file);
        assert_eq!(read, kept);
        assert_eq!(read.get("a"), Some(&result("a")));
        assert_eq!(read.get("b"), None);
    }

    #[test]
    fn a_damaged_file_is_nothing_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("patches.json");
        std::fs::write(&file, "{\"results\":[{\"key\":").unwrap();
        assert_eq!(Results::read(&file), Results::default());
        std::fs::write(&file, "[1,2]").unwrap();
        assert_eq!(Results::read(&file), Results::default());
    }

    #[test]
    fn a_ninth_result_drops_the_oldest_and_a_repeat_moves_up() {
        let mut kept = Results::default();
        for n in 0..9 {
            kept.keep(result(&n.to_string()));
        }
        assert_eq!(kept.results.len(), 8);
        assert!(kept.get("0").is_none());
        assert_eq!(kept.results[0].key, "8");
        kept.keep(result("3"));
        assert_eq!(kept.results.len(), 8);
        assert_eq!(kept.results[0].key, "3");
    }
}
