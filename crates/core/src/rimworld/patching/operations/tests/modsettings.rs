use super::frameworks::*;
use super::*;

fn option(id: &str, default: &str, inside: &[String]) -> String {
    op(
        &msf("PatchOperationModOption"),
        &format!(
            "<id>{id}</id><defaultValue>{default}</defaultValue>{}",
            operations(inside)
        ),
    )
}

#[test]
fn a_mod_option_runs_its_operations_when_enabled_by_default_or_by_a_stored_value() {
    let patch = option("opt", "true", &[add_x()]);
    assert_eq!(
        msf_only(std::slice::from_ref(&patch), None).xml,
        a_ends_with("<x/>")
    );
    let off = msf_file(&[("opt", "False")], &[]);
    let out = msf_only(&[patch], Some(&off));
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><label>a</label></ThingDef>")
    );
    assert!(out.failures.is_empty() && out.unknown.is_empty());
    let default_off = option("opt", "false", &[add_x()]);
    assert_eq!(
        msf_only(std::slice::from_ref(&default_off), None).xml,
        a_with("<ThingDef><defName>A</defName><label>a</label></ThingDef>")
    );
    let on = msf_file(&[("opt", "True")], &[]);
    assert_eq!(msf_only(&[default_off], Some(&on)).xml, a_ends_with("<x/>"));
}

#[test]
fn a_disabled_mod_option_neither_fails_nor_marks_its_targets_unknown() {
    let unknown = li("Some.UnknownOperation", &xpath("Defs/ThingDef"));
    let patch = option("opt", "false", &[remove_missing(), unknown]);
    let out = msf_only(&[patch], None);
    assert!(out.failures.is_empty());
    assert!(out.unknown.is_empty());
}

#[test]
fn an_enabled_mod_option_stops_at_its_first_failure() {
    let patch = option("opt", "true", &[add_x(), remove_missing(), tag("late")]);
    let out = msf_only(&[patch], None);
    assert_eq!(out.xml, a_ends_with("<x/>"));
    assert_eq!(out.failures.len(), 1);
    let Failure::Failed {
        operation, xpath, ..
    } = &out.failures[0]
    else {
        panic!("a failed operation")
    };
    assert_eq!(
        operation,
        "ModSettingsFramework.PatchOperationModOption(count=3, lastFailedOperation=\
         Verse.PatchOperationRemove(Defs/ThingDef[defName=\"zzz\"])) - opt"
    );
    assert_eq!(xpath, "Defs/ThingDef[defName=\"zzz\"]");
}

#[test]
fn a_mod_option_reads_only_its_owner_s_container() {
    let patch = option("opt", "true", &[add_x()]);
    let someone_else = msf_file(&[("opt", "False")], &[]).replace(OWNER, "someone.else");
    assert_eq!(
        msf_only(&[patch], Some(&someone_else)).xml,
        a_ends_with("<x/>")
    );
}

#[test]
fn a_mod_option_whose_mods_are_not_all_missing_runs_and_otherwise_does_nothing() {
    let gated = |mods: &str| {
        op(
            &msf("PatchOperationModOption"),
            &format!(
                "<id>g</id><defaultValue>true</defaultValue><mods>{mods}</mods>{}",
                operations(&[add_x(), remove_missing()])
            ),
        )
    };
    let missing = msf_only(&[gated("<li>Not Here</li>")], None);
    assert!(missing.failures.is_empty());
    assert_eq!(
        missing.xml,
        a_with("<ThingDef><defName>A</defName><label>a</label></ThingDef>")
    );
    let any = msf_only(&[gated("<li>Not Here</li><li>Here Mod</li>")], None);
    assert_eq!(any.failures.len(), 1);
    assert!(any.xml.contains("<x/>"));
}

#[test]
fn radio_buttons_run_every_enabled_option_and_always_succeed() {
    let radio = op(
        &msf("PatchOperationRadioButtons"),
        &format!(
            "<options>{}{}{}</options>",
            li(
                &msf("PatchOperationModOption"),
                &format!(
                    "<id>a</id><defaultValue>true</defaultValue>{}",
                    operations(&[remove_missing()])
                )
            ),
            li(
                &msf("PatchOperationModOption"),
                &format!(
                    "<id>b</id><defaultValue>false</defaultValue>{}",
                    operations(&[tag("b")])
                )
            ),
            li(
                &msf("PatchOperationModOption"),
                &format!(
                    "<id>c</id><defaultValue>true</defaultValue>{}",
                    operations(&[tag("c")])
                )
            ),
        ),
    );
    let out = msf_only(&[radio], None);
    assert!(out.failures.is_empty());
    assert_eq!(out.xml, a_ends_with("<c/>"));
}

fn slider(class: &str, default: &str, of: &str) -> String {
    op(
        &msf(class),
        &format!(
            "<id>s</id><defaultValue>{default}</defaultValue>{}",
            xpath(of)
        ),
    )
}

#[test]
fn a_float_slider_sets_the_text_of_its_first_match_and_succeeds_without_one() {
    let patch = slider("PatchOperationSliderFloat", "2.5", "Defs/ThingDef/label");
    let out = msf_only(std::slice::from_ref(&patch), None);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><label>2.5</label></ThingDef>")
    );
    assert_eq!(out.traces[0].wrote, BTreeSet::from([key("A")]));
    let stored = msf_file(&[], &[("s", "-4")]);
    let out = msf_only(&[patch], Some(&stored));
    assert!(out.xml.contains("<label>-4</label>"));
    let none = slider("PatchOperationSliderFloat", "1", "Defs/Nope");
    let out = msf_only(&[none], None);
    assert!(out.failures.is_empty());
}

#[test]
fn an_int_slider_without_a_match_fails() {
    let none = slider("PatchOperationSliderInt", "1", "Defs/Nope");
    assert_eq!(msf_only(&[none], None).failures.len(), 1);
    let some = slider("PatchOperationSliderInt", "7", "Defs/ThingDef/label");
    assert!(msf_only(&[some], None).xml.contains("<label>7</label>"));
}

#[test]
fn a_damaged_settings_file_means_the_defaults() {
    let patch = option("opt", "true", &[add_x()]);
    let out = msf_only(&[patch], Some("<SettingsBlock><ModSettings>"));
    assert_eq!(out.xml, a_ends_with("<x/>"));
    let out = xe_only(
        &[optional("on", "true", &tag("yes"), &tag("no"))],
        Some("not xml"),
    );
    assert_eq!(out.xml, a_ends_with("<yes/>"));
}

#[test]
fn an_unknown_framework_operation_stays_unknown() {
    let worker = op(&msf("PatchOperationWorker"), &xpath("Defs/ThingDef"));
    assert_eq!(msf_only(&[worker], None).unknown.len(), 1);
}
