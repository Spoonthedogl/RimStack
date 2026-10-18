//! ModSettingsFramework's patch operations (workshop 2260097569), resolved while reading from the
//! stored values of the mod whose `Patches` hold them, else from their own defaults.

use super::read::{
    Listed, Operation, Reading, Work, child, operation, success_of, text_of, wanted,
};
use super::xmlextensions::nested;
use libxml::tree::Node;

const CLASS: &str = "ModSettingsFramework.";
const OPTION: &str = "ModSettingsFramework.PatchOperationModOption";

pub(super) fn read(class: &str, node: &Node, reading: &Reading) -> Option<Work> {
    Some(match class.strip_prefix(CLASS)? {
        "PatchOperationModOption" => option(node, reading),
        "PatchOperationRadioButtons" => radio(node, reading),
        "PatchOperationSliderFloat" => slider(node, reading, false),
        "PatchOperationSliderInt" => slider(node, reading, true),
        _ => return None,
    })
}

/// `CanRun`: no `mods`, or one of them active by its `About.xml` name. A category counts for nothing,
/// since its def is not loaded while patching.
fn can_run(node: &Node, reading: &Reading) -> bool {
    let Some(mods) = child(node, "mods") else {
        return true;
    };
    let names: Vec<String> = mods
        .get_child_elements()
        .iter()
        .map(|li| li.get_content().trim().to_string())
        .collect();
    names.is_empty() || names.iter().any(|name| reading.active.has_name(name))
}

fn boolean(node: &Node, name: &str) -> bool {
    text_of(node, name).is_some_and(|t| t.eq_ignore_ascii_case("true"))
}

fn option(node: &Node, reading: &Reading) -> Work {
    let id = text_of(node, "id");
    let all: Vec<Operation> = child(node, "operations")
        .map(|list| list.get_child_elements())
        .unwrap_or_default()
        .iter()
        .filter(|li| wanted(li, reading.active))
        .map(|li| operation(li, reading))
        .collect();
    let listed = Listed {
        count: all.len(),
        id: id.clone().unwrap_or_default(),
    };
    if !can_run(node, reading) {
        return Work::Nested {
            operations: Vec::new(),
            listed: Some(listed),
        };
    }
    let Some(id) = id else {
        return Work::Fails;
    };
    let stored = reading.settings.enabled(reading.owner, &id);
    let enabled = stored.unwrap_or_else(|| boolean(node, "defaultValue"));
    Work::Nested {
        operations: if enabled { all } else { Vec::new() },
        listed: Some(listed),
    }
}

fn radio(node: &Node, reading: &Reading) -> Work {
    if !can_run(node, reading) {
        return nested(Vec::new());
    }
    let options = child(node, "options")
        .map(|list| list.get_child_elements())
        .unwrap_or_default();
    Work::Each(
        options
            .iter()
            .map(|li| Operation {
                class: li
                    .get_attribute("Class")
                    .unwrap_or_else(|| OPTION.to_string()),
                success: success_of(li),
                work: option(li, reading),
            })
            .collect(),
    )
}

fn slider(node: &Node, reading: &Reading, whole: bool) -> Work {
    if !can_run(node, reading) {
        return nested(Vec::new());
    }
    let (Some(id), Some(xpath)) = (text_of(node, "id"), text_of(node, "xpath")) else {
        return Work::Fails;
    };
    let default = text_of(node, "defaultValue").unwrap_or_default();
    let default = if whole {
        default.parse::<i32>().map_or(0.0, |d| d as f32)
    } else {
        default.parse::<f32>().unwrap_or(0.0)
    };
    let value = reading
        .settings
        .value(reading.owner, &id)
        .unwrap_or(default);
    Work::Slider {
        xpath: Some(xpath),
        text: value.to_string(),
        strict: whole,
    }
}
