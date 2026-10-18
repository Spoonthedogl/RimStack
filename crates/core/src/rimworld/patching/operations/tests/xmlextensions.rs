use super::frameworks::*;
use super::*;

#[test]
fn optional_patch_takes_the_branch_of_the_stored_value() {
    let patch = optional("on", "true", &tag("yes"), &tag("no"));
    let stored = xe_file(&[("Own.Mod;on", "False")]);
    let out = xe_only(std::slice::from_ref(&patch), Some(&stored));
    assert_eq!(out.xml, a_ends_with("<no/>"));
    assert!(out.failures.is_empty() && out.unknown.is_empty());
    let out = xe_only(&[patch], Some(&xe_file(&[("Own.Mod;on", "true")])));
    assert_eq!(out.xml, a_ends_with("<yes/>"));
}

#[test]
fn optional_patch_takes_the_branch_of_its_default_without_a_stored_value() {
    let patch = optional("on", "true", &tag("yes"), &tag("no"));
    assert_eq!(
        xe_only(std::slice::from_ref(&patch), None).xml,
        a_ends_with("<yes/>")
    );
    let other = xe_file(&[("Own.Mod;other", "false"), ("Else;on", "false")]);
    assert_eq!(xe_only(&[patch], Some(&other)).xml, a_ends_with("<yes/>"));
}

#[test]
fn optional_patch_with_a_branch_missing_succeeds_and_one_that_is_no_bool_fails() {
    let only_true = op(
        &xe("OptionalPatch"),
        "<modId>M</modId><key>k</key><defaultValue>false</defaultValue><caseTrue></caseTrue>",
    );
    let out = xe_only(&[only_true], None);
    assert!(out.failures.is_empty());
    let nonsense = optional("k", "maybe", "", "");
    let out = xe_only(&[nonsense], None);
    assert_eq!(out.failures.len(), 1);
    let Failure::Failed { operation, .. } = &out.failures[0] else {
        panic!("a failed operation")
    };
    assert_eq!(operation, "XmlExtensions.OptionalPatch");
}

#[test]
fn optional_patch_with_keys_combines_them_by_its_logic() {
    let many = |logic: &str| {
        op(
            &xe("OptionalPatch"),
            &format!(
                "<modId>M</modId><keys><li>a</li><li>b</li></keys><defaultValues><li>true</li><li>false</li></defaultValues>\
                 <logic>{logic}</logic><caseTrue>{}</caseTrue><caseFalse>{}</caseFalse>",
                tag("yes"),
                tag("no")
            ),
        )
    };
    assert_eq!(xe_only(&[many("And")], None).xml, a_ends_with("<no/>"));
    assert_eq!(xe_only(&[many("Or")], None).xml, a_ends_with("<yes/>"));
    let uneven = op(
        &xe("OptionalPatch"),
        "<modId>M</modId><keys><li>a</li></keys><defaultValues></defaultValues>",
    );
    assert_eq!(xe_only(&[uneven], None).failures.len(), 1);
}

#[test]
fn use_setting_substitutes_into_attributes_and_text() {
    let patch = op(
        &xe("UseSetting"),
        &format!(
            "<modId>Own.Mod</modId><key>part</key><defaultValue>wing</defaultValue><apply>{}</apply>",
            li(
                "PatchOperationAdd",
                "<xpath>Defs/ThingDef[defName=\"A\"]</xpath><value><n Kind=\"k_{part}\">t {part}</n></value>"
            )
        ),
    );
    let out = xe_only(std::slice::from_ref(&patch), None);
    assert_eq!(out.xml, a_ends_with("<n Kind=\"k_wing\">t wing</n>"));
    let stored = xe_file(&[("Own.Mod;part", "leg")]);
    let out = xe_only(&[patch], Some(&stored));
    assert_eq!(out.xml, a_ends_with("<n Kind=\"k_leg\">t leg</n>"));
}

#[test]
fn use_setting_substitutes_into_tags_when_the_brackets_are_name_characters() {
    let patch = op(
        &xe("UseSetting"),
        &format!(
            "<modId>M</modId><key>part</key><defaultValue>wing</defaultValue><brackets>__</brackets><apply>{}</apply>",
            li(
                "PatchOperationAdd",
                &format!("{}<value><_part_/></value>", xpath_a(""))
            )
        ),
    );
    assert_eq!(xe_only(&[patch], None).xml, a_ends_with("<wing/>"));
}

#[test]
fn use_settings_and_brackets_substitute_each_key() {
    let patch = op(
        &xe("UseSettings"),
        &format!(
            "<modId>M</modId><keys><li>a</li><li>b</li></keys><defaultValues><li>1</li><li>2</li></defaultValues>\
             <brackets>[]</brackets><apply>{}</apply>",
            li(
                "PatchOperationAdd",
                &format!("{}<value><n>[a]-[b]-{{a}}</n></value>", xpath_a(""))
            )
        ),
    );
    let out = xe_only(&[patch], Some(&xe_file(&[("M;b", "9")])));
    assert_eq!(out.xml, a_ends_with("<n>1-9-{a}</n>"));
}

#[test]
fn a_failing_inner_operation_of_use_setting_fails_it_and_a_missing_key_does_too() {
    let inside = op(
        &xe("UseSetting"),
        &format!(
            "<modId>M</modId><key>k</key><defaultValue>1</defaultValue><apply>{}{}</apply>",
            add_x(),
            remove_missing()
        ),
    );
    let out = xe_only(&[inside], None);
    assert_eq!(out.xml, a_ends_with("<x/>"));
    assert_eq!(out.failures.len(), 1);
    let no_key = op(&xe("UseSetting"), "<modId>M</modId><apply></apply>");
    assert_eq!(xe_only(&[no_key], None).failures.len(), 1);
}

const WITH_PATCH_DEF: &str = "<ThingDef><defName>A</defName><label>a</label></ThingDef>\
     <ThingDef><defName>B</defName></ThingDef>\
     <XmlExtensions.PatchDef Name=\"AddTo\"><parameters><li>who</li><li>what</li></parameters>\
     <apply><li Class=\"PatchOperationAdd\"><xpath>Defs/ThingDef[defName=\"{who}\"]</xpath>\
     <value><tag>{what}</tag></value></li></apply></XmlExtensions.PatchDef>";

fn apply_patch(name: &str, arguments: &[&str]) -> String {
    let list: String = arguments.iter().map(|a| format!("<li>{a}</li>")).collect();
    op(
        &xe("ApplyPatch"),
        &format!("<patchName>{name}</patchName><arguments>{list}</arguments>"),
    )
}

#[test]
fn apply_patch_runs_a_patch_def_with_its_arguments() {
    let out = play(
        WITH_PATCH_DEF,
        &[apply_patch("AddTo", &["A", "z"])],
        None,
        None,
    );
    assert!(out.xml.contains("<label>a</label><tag>z</tag></ThingDef>"));
    assert!(out.failures.is_empty() && out.unknown.is_empty());
    assert_eq!(out.traces[0].wrote, BTreeSet::from([key("A")]));
}

#[test]
fn apply_patch_fails_on_a_wrong_count_or_an_unknown_name() {
    let few = play(WITH_PATCH_DEF, &[apply_patch("AddTo", &["A"])], None, None);
    let none = play(WITH_PATCH_DEF, &[apply_patch("Nope", &[])], None, None);
    assert_eq!(few.failures.len(), 1);
    assert_eq!(none.failures.len(), 1);
    assert!(!few.xml.contains("<label>a</label><tag>"));
}

#[test]
fn apply_patch_inside_use_setting_takes_the_substituted_arguments() {
    let patch = op(
        &xe("UseSetting"),
        &format!(
            "<modId>M</modId><key>tag</key><defaultValue>q</defaultValue><apply>{}</apply>",
            li(
                &xe("ApplyPatch"),
                "<patchName>AddTo</patchName><arguments><li>A</li><li>{tag}</li></arguments>"
            )
        ),
    );
    let out = play(WITH_PATCH_DEF, &[patch], None, None);
    assert!(out.xml.contains("<label>a</label><tag>q</tag></ThingDef>"));
}

#[test]
fn a_patch_def_that_applies_itself_ends() {
    let looping = "<XmlExtensions.PatchDef Name=\"Loop\"><apply>\
        <li Class=\"XmlExtensions.ApplyPatch\"><patchName>Loop</patchName></li></apply></XmlExtensions.PatchDef>";
    let out = play(looping, &[apply_patch("Loop", &[])], None, None);
    assert_eq!(out.failures.len(), 1);
}

#[test]
fn safe_remove_succeeds_when_nothing_matches_but_remove_does_not() {
    let safe = op(
        &xe("PatchOperationSafeRemove"),
        &xpath("Defs/ThingDef[defName=\"zzz\"]"),
    );
    let plain = op(
        &xe("PatchOperationRemove"),
        &xpath("Defs/ThingDef[defName=\"zzz\"]"),
    );
    let out = xe_only(&[safe], None);
    assert!(out.failures.is_empty());
    let out = xe_only(&[plain], None);
    assert_eq!(out.failures.len(), 1);
    let real = op(
        &xe("PatchOperationSafeRemove"),
        &xpath("Defs/ThingDef/label"),
    );
    let out = xe_only(&[real], None);
    assert_eq!(out.xml, a_with("<ThingDef><defName>A</defName></ThingDef>"));
    assert_eq!(out.traces[0].wrote, BTreeSet::from([key("A")]));
}

#[test]
fn the_plain_xmlextensions_operations_do_what_vanilla_ones_do() {
    let add = op(
        &xe("PatchOperationAdd"),
        &format!("{}<value><x/><y/></value>", xpath_a("")),
    );
    let prepend = op(
        &xe("PatchOperationAdd"),
        &format!(
            "{}<order>Prepend</order><value><p/><q/></value>",
            xpath_a("")
        ),
    );
    let out = xe_only(&[add, prepend], None);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><p/><q/><defName>A</defName><label>a</label><x/><y/></ThingDef>")
    );
    let after = op(
        &xe("PatchOperationInsert"),
        &format!(
            "{}<order>Append</order><value><i/><j/></value>",
            xpath_a("/label")
        ),
    );
    let out = xe_only(&[after], None);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><label>a</label><j/><i/></ThingDef>")
    );
    let replace = op(
        &xe("PatchOperationReplace"),
        &format!("{}<value><name>n</name></value>", xpath_a("/label")),
    );
    let out = xe_only(&[replace], None);
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><name>n</name></ThingDef>")
    );
}

#[test]
fn xmlextensions_set_name_drops_the_attributes() {
    let defs = "<ThingDef Name=\"N\"><defName>A</defName></ThingDef>";
    let patch = op(
        &xe("PatchOperationSetName"),
        "<xpath>Defs/ThingDef[@Name=\"N\"]</xpath><name>RecipeDef</name>",
    );
    let out = play(defs, &[patch], None, None);
    assert_eq!(
        out.xml,
        "<Defs><RecipeDef><defName>A</defName></RecipeDef></Defs>"
    );
}

#[test]
fn select_single_node_limits_a_plain_operation_to_the_first_match() {
    let patch = op(
        &xe("PatchOperationAdd"),
        "<xpath>Defs/ThingDef</xpath><selectSingleNode>true</selectSingleNode><value><x/></value>",
    );
    let out = xe_only(&[patch], None);
    assert_eq!(
        out.xml,
        "<Defs><ThingDef><defName>A</defName><label>a</label><x/></ThingDef>\
         <ThingDef><defName>B</defName></ThingDef></Defs>"
    );
}

fn safe(class: &str, extra: &str, value: &str) -> String {
    op(
        &xe(class),
        &format!(
            "{}{extra}<value>{value}</value>",
            xpath("Defs/ThingDef[defName=\"A\"]")
        ),
    )
}

#[test]
fn safe_add_appends_only_what_is_not_there() {
    let out = xe_only(
        &[safe(
            "PatchOperationSafeAdd",
            "",
            "<label>other</label><x/>",
        )],
        None,
    );
    assert_eq!(out.xml, a_ends_with("<x/>"));
}

#[test]
fn safe_add_merges_into_a_node_that_is_there() {
    let defs = "<ThingDef><defName>A</defName><stats><a>1</a></stats></ThingDef>";
    let patch = safe(
        "PatchOperationSafeAdd",
        "",
        "<stats><a>2</a><b>3</b></stats>",
    );
    let out = play(defs, &[patch], None, None);
    assert_eq!(
        out.xml,
        "<Defs><ThingDef><defName>A</defName><stats><a>1</a><b>3</b></stats></ThingDef></Defs>"
    );
}

#[test]
fn safe_replace_replaces_what_is_there_and_adds_nothing() {
    let out = xe_only(
        &[safe(
            "PatchOperationSafeReplace",
            "",
            "<label>new</label><x/>",
        )],
        None,
    );
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><label>new</label></ThingDef>")
    );
}

#[test]
fn safe_add_or_replace_does_both() {
    let out = xe_only(
        &[safe(
            "PatchOperationSafeAddOrReplace",
            "",
            "<label>new</label><x/>",
        )],
        None,
    );
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><label>new</label><x/></ThingDef>")
    );
    let out = xe_only(
        &[safe(
            "PatchOperationAddOrReplace",
            "",
            "<label>new</label><x/>",
        )],
        None,
    );
    assert_eq!(
        out.xml,
        a_with("<ThingDef><defName>A</defName><label>new</label><x/></ThingDef>")
    );
}

#[test]
fn a_safe_operation_with_no_match_fails() {
    let patch = op(
        &xe("PatchOperationSafeAdd"),
        &format!(
            "{}<value><x/></value>",
            xpath("Defs/ThingDef[defName=\"zzz\"]")
        ),
    );
    assert_eq!(xe_only(&[patch], None).failures.len(), 1);
}

#[test]
fn a_safe_comparison_by_inner_text_and_attributes() {
    let defs = "<ThingDef><defName>A</defName><li K=\"1\">p</li><li K=\"2\">q</li></ThingDef>";
    let by_text = safe(
        "PatchOperationSafeAdd",
        "<compare>InnerText</compare>",
        "<li K=\"9\">q</li><li K=\"9\">r</li>",
    );
    let out = play(defs, &[by_text], None, None);
    assert_eq!(
        out.xml,
        "<Defs><ThingDef><defName>A</defName><li K=\"1\">p</li><li K=\"2\">q</li><li K=\"9\">r</li></ThingDef></Defs>"
    );
    let by_attributes = safe(
        "PatchOperationSafeAdd",
        "<checkAttributes>true</checkAttributes>",
        "<li K=\"1\">z</li><li K=\"3\">z</li>",
    );
    let out = play(defs, &[by_attributes], None, None);
    assert!(out.xml.ends_with("<li K=\"3\">z</li></ThingDef></Defs>"));
    assert_eq!(out.xml.matches("K=\"1\"").count(), 1);
}

#[test]
fn xmlextensions_find_mod_and_conditional_choose_a_branch() {
    let find = |mods: &str, extra: &str| {
        op(
            &xe("FindMod"),
            &format!(
                "<mods>{mods}</mods>{extra}<caseTrue>{}</caseTrue><caseFalse>{}</caseFalse>",
                tag("yes"),
                tag("no")
            ),
        )
    };
    let out = xe_only(&[find("<li>here mod</li><li>nope</li>", "")], None);
    assert_eq!(out.xml, a_ends_with("<yes/>"));
    let out = xe_only(
        &[find("<li>here mod</li><li>nope</li>", "<logic>and</logic>")],
        None,
    );
    assert_eq!(out.xml, a_ends_with("<no/>"));
    let out = xe_only(
        &[find("<li>HERE.mod</li>", "<packageId>true</packageId>")],
        None,
    );
    assert_eq!(out.xml, a_ends_with("<yes/>"));
    let conditional = op(
        &xe("Conditional"),
        &format!("{}<caseTrue>{}</caseTrue>", xpath_a("/label"), tag("seen")),
    );
    let out = xe_only(&[conditional], None);
    assert_eq!(out.xml, a_ends_with("<seen/>"));
    let missing = op(
        &xe("Conditional"),
        &format!("{}<caseTrue>{}</caseTrue>", xpath("Defs/Nope"), tag("seen")),
    );
    let out = xe_only(&[missing], None);
    assert!(
        out.failures.is_empty()
            && out.xml == a_with("<ThingDef><defName>A</defName><label>a</label></ThingDef>")
    );
}

#[test]
fn xmlextensions_sequence_stops_at_the_first_failure() {
    let patch = op(
        &xe("Sequence"),
        &format!(
            "<apply>{}{}{}</apply>",
            tag("one"),
            remove_missing(),
            tag("two")
        ),
    );
    let out = xe_only(&[patch], None);
    assert_eq!(out.xml, a_ends_with("<one/>"));
    assert_eq!(out.failures.len(), 1);
}

#[test]
fn success_applies_to_xmlextensions_operations() {
    let inverted = op(
        &xe("PatchOperationSafeRemove"),
        &format!("{}<success>Invert</success>", xpath("Defs/Nope")),
    );
    assert_eq!(xe_only(&[inverted], None).failures.len(), 1);
    let always = op(
        &xe("PatchOperationRemove"),
        &format!("{}<success>Always</success>", xpath("Defs/Nope")),
    );
    assert!(xe_only(&[always], None).failures.is_empty());
}

#[test]
fn what_xmlextensions_cannot_be_simulated_stays_unknown_and_only_the_taken_branch_counts() {
    let external = op(
        &xe("OptionalPatchExternal"),
        &format!("<field>f</field><caseTrue>{}</caseTrue>", tag("a")),
    );
    let out = xe_only(&[external], None);
    assert_eq!(out.unknown.len(), 1);
    let guarded = optional(
        "on",
        "false",
        &li(
            &xe("DefDatabaseOperationRemove"),
            &xpath("Defs/ThingDef[defName=\"A\"]"),
        ),
        &tag("fine"),
    );
    let out = xe_only(&[guarded], None);
    assert!(out.unknown.is_empty(), "{:?}", out.unknown);
    let in_document = op(
        &xe("PatchOperationSafeRemove"),
        &format!("{}<xmlDoc>Other</xmlDoc>", xpath("Defs/ThingDef")),
    );
    let out = xe_only(&[in_document], None);
    assert_eq!(out.unknown.len(), 1);
    assert!(out.xml.contains("<label>a</label>"));
}
