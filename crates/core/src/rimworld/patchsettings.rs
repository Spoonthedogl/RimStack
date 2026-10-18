//! What the user stored for the two patch frameworks, in the game's `Config` folder; read-only.
//! XmlExtensions: `Mod_2574315206_XmlMod.xml`, a `dataDict` of `modId;key` to text.
//! ModSettingsFramework: one file for every mod, a container per owning mod's lowercase package id.

use libxml::parser::{Parser, ParserOptions};
use libxml::tree::Node;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const XML_EXTENSIONS: &str = "Mod_2574315206_XmlMod.xml";
const FRAMEWORK: &str = "ModSettingsFrameworkMod_Settings.xml";

#[derive(Clone, Debug, Default, PartialEq)]
struct Container {
    states: HashMap<String, bool>,
    values: HashMap<String, f32>,
}

/// A missing or damaged file is the same as an empty one: every operation then uses its own default.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    xml_extensions: HashMap<String, String>,
    framework: HashMap<String, Container>,
    owners: Vec<Option<String>>,
}

impl Settings {
    /// `config` is the folder holding `Config/`.
    pub fn read(config: Option<&Path>) -> Settings {
        let [xml_extensions, framework] = Settings::files(config);
        Settings {
            xml_extensions: parsed(&xml_extensions, data_dict).unwrap_or_default(),
            framework: parsed(&framework, containers).unwrap_or_default(),
            owners: Vec::new(),
        }
    }

    /// The files `read` looks at, for a key that must change when one does.
    pub fn files(config: Option<&Path>) -> [PathBuf; 2] {
        let folder = config.map(|c| c.join("Config")).unwrap_or_default();
        [folder.join(XML_EXTENSIONS), folder.join(FRAMEWORK)]
    }

    /// The package id of each place's mod, as `Mod::key` gives it (lowercase), for the framework's containers.
    pub fn owned_by(mut self, owners: Vec<Option<String>>) -> Settings {
        self.owners = owners;
        self
    }

    pub fn owner(&self, place: usize) -> Option<&str> {
        self.owners.get(place)?.as_deref()
    }

    pub fn xml_extensions(&self, mod_id: &str, key: &str) -> Option<&str> {
        let stored = self.xml_extensions.get(&format!("{mod_id};{key}"));
        stored.map(String::as_str)
    }

    pub fn enabled(&self, owner: Option<&str>, id: &str) -> Option<bool> {
        self.container(owner)?.states.get(id).copied()
    }

    pub fn value(&self, owner: Option<&str>, id: &str) -> Option<f32> {
        self.container(owner)?.values.get(id).copied()
    }

    fn container(&self, owner: Option<&str>) -> Option<&Container> {
        self.framework.get(&owner?.to_lowercase())
    }
}

fn parsed<T>(path: &Path, read: impl FnOnce(&Node) -> T) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim_start_matches('\u{feff}').to_string();
    let strict = ParserOptions {
        recover: false,
        ..ParserOptions::default()
    };
    let tree = Parser::default()
        .parse_string_with_options(text, strict)
        .ok()?;
    Some(read(&tree.get_root_element()?))
}

fn child(node: &Node, name: &str) -> Option<Node> {
    node.get_child_elements()
        .into_iter()
        .find(|c| c.get_name() == name)
}

/// Scribe's dictionary: parallel `keys` and `values` lists of `li`.
fn pairs(node: &Node) -> Vec<(String, Node)> {
    let keys = child(node, "keys").map(|k| k.get_child_elements());
    let values = child(node, "values").map(|v| v.get_child_elements());
    let (Some(keys), Some(values)) = (keys, values) else {
        return Vec::new();
    };
    keys.iter()
        .map(|k| k.get_content().trim().to_string())
        .zip(values)
        .collect()
}

fn data_dict(root: &Node) -> HashMap<String, String> {
    let dict = child(root, "ModSettings").and_then(|m| child(&m, "dataDict"));
    let entries = dict.map(|d| pairs(&d)).unwrap_or_default();
    entries
        .into_iter()
        .map(|(key, value)| (key, value.get_content()))
        .collect()
}

fn containers(root: &Node) -> HashMap<String, Container> {
    let all = child(root, "ModSettings").and_then(|m| child(&m, "modSettingsPerModId"));
    let entries = all.map(|a| pairs(&a)).unwrap_or_default();
    entries
        .into_iter()
        .map(|(package, node)| (package.to_lowercase(), container(&node)))
        .collect()
}

fn container(node: &Node) -> Container {
    let states = child(node, "patchOperationStates").map(|s| pairs(&s));
    let values = child(node, "patchOperationValues").map(|v| pairs(&v));
    Container {
        states: states
            .into_iter()
            .flatten()
            .filter_map(|(id, v)| Some((id, v.get_content().trim().to_lowercase().parse().ok()?)))
            .collect(),
        values: values
            .into_iter()
            .flatten()
            .filter_map(|(id, v)| Some((id, v.get_content().trim().parse().ok()?)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(xml_extensions: Option<&str>, framework: Option<&str>) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Config")).unwrap();
        for (name, text) in [(XML_EXTENSIONS, xml_extensions), (FRAMEWORK, framework)] {
            if let Some(text) = text {
                std::fs::write(tmp.path().join("Config").join(name), text).unwrap();
            }
        }
        tmp
    }

    #[test]
    fn stored_values_are_found_by_their_keys() {
        let tmp = config(
            Some(
                "\u{feff}<SettingsBlock><ModSettings Class=\"X\"><dataDict><keys><li>A.B;on</li><li>A.B;n</li></keys>\
                 <values><li>false</li><li>3</li></values></dataDict></ModSettings></SettingsBlock>",
            ),
            Some(
                "<SettingsBlock><ModSettings><modSettingsPerModId><keys><li>Own.Er</li></keys><values><li>\
                 <packageID>own.er</packageID><patchOperationStates><keys><li>opt</li></keys><values><li>True</li></values>\
                 </patchOperationStates><patchOperationValues><keys><li>s</li></keys><values><li>-4</li></values>\
                 </patchOperationValues></li></values></modSettingsPerModId></ModSettings></SettingsBlock>",
            ),
        );
        let settings = Settings::read(Some(tmp.path()));
        assert_eq!(settings.xml_extensions("A.B", "on"), Some("false"));
        assert_eq!(settings.xml_extensions("A.B", "n"), Some("3"));
        assert_eq!(settings.xml_extensions("A.B", "other"), None);
        assert_eq!(settings.enabled(Some("OWN.er"), "opt"), Some(true));
        assert_eq!(settings.value(Some("own.er"), "s"), Some(-4.0));
        assert_eq!(settings.enabled(Some("other"), "opt"), None);
        assert_eq!(settings.enabled(None, "opt"), None);
    }

    #[test]
    fn a_missing_or_damaged_file_stores_nothing() {
        let tmp = config(Some("<SettingsBlock><dataDict>"), None);
        let settings = Settings::read(Some(tmp.path()));
        assert_eq!(settings.xml_extensions("A", "k"), None);
        assert_eq!(settings.enabled(Some("a"), "k"), None);
        assert_eq!(Settings::read(None).xml_extensions("A", "k"), None);
    }
}
