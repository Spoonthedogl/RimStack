use super::*;

pub(super) const OWNER: &str = "own.er";

pub(super) fn li(class: &str, body: &str) -> String {
    format!("<li Class=\"{class}\">{body}</li>")
}

pub(super) fn xe(class: &str) -> String {
    format!("XmlExtensions.{class}")
}

pub(super) fn xpath(of: &str) -> String {
    format!("<xpath>{of}</xpath>")
}

pub(super) fn add_x() -> String {
    li(
        "PatchOperationAdd",
        &format!("{}<value><x/></value>", xpath_a("")),
    )
}

pub(super) fn remove_missing() -> String {
    li(
        "PatchOperationRemove",
        &xpath("Defs/ThingDef[defName=\"zzz\"]"),
    )
}

pub(super) fn xe_file(entries: &[(&str, &str)]) -> String {
    let keys: String = entries
        .iter()
        .map(|(k, _)| format!("<li>{k}</li>"))
        .collect();
    let values: String = entries
        .iter()
        .map(|(_, v)| format!("<li>{v}</li>"))
        .collect();
    format!(
        "<SettingsBlock><ModSettings Class=\"XmlExtensions.XmlModBaseSettings\"><dataDict><keys>{keys}</keys>\
         <values>{values}</values></dataDict></ModSettings></SettingsBlock>"
    )
}

pub(super) fn settings_in(
    xml_extensions: Option<&str>,
    framework: Option<&str>,
) -> (tempfile::TempDir, Settings) {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("Config")).unwrap();
    let [first, second] = Settings::files(Some(tmp.path()));
    for (file, text) in [(first, xml_extensions), (second, framework)] {
        if let Some(text) = text {
            fs::write(file, text).unwrap();
        }
    }
    let settings = Settings::read(Some(tmp.path())).owned_by(vec![Some(OWNER.into())]);
    (tmp, settings)
}

pub(super) fn play(
    defs: &str,
    ops: &[String],
    xml_extensions: Option<&str>,
    framework: Option<&str>,
) -> Outcome {
    let (_tmp, settings) = settings_in(xml_extensions, framework);
    let patch = format!("<Patch>{}</Patch>", ops.concat());
    run_mods_with(
        defs,
        &[&patch],
        &active(&["here.mod"], &["Here Mod"]),
        &settings,
        &[0],
    )
}

pub(super) fn xe_only(ops: &[String], xml_extensions: Option<&str>) -> Outcome {
    play(DEFS, ops, xml_extensions, None)
}

pub(super) fn tag(name: &str) -> String {
    li(
        "PatchOperationAdd",
        &format!("{}<value><{name}/></value>", xpath_a("")),
    )
}

pub(super) fn a_ends_with(extra: &str) -> String {
    a_with(&format!(
        "<ThingDef><defName>A</defName><label>a</label>{extra}</ThingDef>"
    ))
}

pub(super) fn optional(key: &str, default: &str, yes: &str, no: &str) -> String {
    op(
        &xe("OptionalPatch"),
        &format!(
            "<modId>Own.Mod</modId><key>{key}</key><defaultValue>{default}</defaultValue>\
             <caseTrue>{yes}</caseTrue><caseFalse>{no}</caseFalse>"
        ),
    )
}
