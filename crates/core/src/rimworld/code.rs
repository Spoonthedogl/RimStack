use crate::rimworld::dotnet::{Assembly, Does, Patch, Type};
use crate::rimworld::game::{Game, When, drawing};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Extending these says nothing about what a mod is for.
const MOD_ITSELF: [&str; 2] = ["Verse.Mod", "Verse.ModSettings"];

/// Extending these adds nothing to the game's world.
const HOOKS: [&str; 4] = [
    "Verse.GameComponent",
    "Verse.MapComponent",
    "RimWorld.Planet.WorldComponent",
    "Verse.Def",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aim {
    pub target: String,
    pub method: String,
    /// Empty for a patch applied by hand: the code that applies it is not followed.
    pub does: Does,
    /// `None` if the method is not the game's, or nothing the engine calls leads to it.
    pub when: Option<When>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    pub types: usize,
    /// Not counting the settings page.
    pub drawing: usize,
    pub adding: usize,
    pub patches: usize,
    pub on_playing: usize,
    pub on_interface: usize,
    pub on_loading: usize,
    pub replacing: usize,
}

impl Facts {
    pub fn is_interface(&self) -> bool {
        let mostly = |part: usize, whole: usize| whole > 0 && part * 2 >= whole;
        let draws_more = self.drawing > 0 && self.drawing >= self.adding * 2;
        let much_of_it = self.drawing * 5 >= self.types || mostly(self.on_interface, self.patches);
        draws_more && much_of_it
    }

    /// Performance mods count here too: a faster method and a changed one are the same patch.
    pub fn is_tweak(&self) -> bool {
        self.patches >= 3 && self.on_playing * 2 >= self.patches && self.adding == 0
    }

    fn add_types(&mut self, types: &[Type], game: &Game) {
        let draws = drawing(types, game.drawing_types());
        let by_name: HashMap<&str, &Type> = types.iter().map(|t| (t.name.as_str(), t)).collect();
        let root = |t: &Type| -> Option<String> {
            let mut parent = t.extends.clone()?;
            for _ in 0..32 {
                match by_name.get(parent.as_str()) {
                    Some(own) => parent = own.extends.clone()?,
                    None => break,
                }
            }
            Some(parent)
        };
        for t in types {
            self.types += 1;
            let from = root(t);
            let from = from.as_deref().unwrap_or_default();
            if MOD_ITSELF.contains(&from) {
                continue;
            }
            if draws.contains(&t.name) {
                self.drawing += 1;
            } else if game.has(from) && !HOOKS.contains(&from) {
                self.adding += 1;
            }
        }
    }

    fn add_aim(&mut self, aim: &Aim) {
        self.patches += 1;
        match aim.when {
            Some(When::Playing) => self.on_playing += 1,
            Some(When::Interface) => self.on_interface += 1,
            Some(When::Loading) => self.on_loading += 1,
            None => {}
        }
        self.replacing += usize::from(aim.does.prefix || aim.does.transpiler);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Code {
    pub facts: Facts,
    /// Sorted by target, then method; each method once.
    pub aims: Vec<Aim>,
}

impl Code {
    /// Looked-up members aim with no patch kinds.
    pub fn add(
        &mut self,
        types: &[Type],
        patches: &[Patch],
        looked_up: &[(String, String)],
        hand_applied: &[Patch],
        game: &Game,
    ) {
        self.facts.add_types(types, game);
        for p in patches {
            let when = game.method(&p.target, &p.method).and_then(|runs| runs.when);
            self.aim(&p.target, &p.method, p.does, when);
        }
        for (target, method) in looked_up {
            if let Some(runs) = game.method(target, method) {
                self.aim(target, method, Does::default(), runs.when);
            }
        }
        for p in hand_applied {
            if let Some(runs) = game.method(&p.target, &p.method) {
                self.aim(&p.target, &p.method, p.does, runs.when);
            }
        }
    }

    fn aim(&mut self, target: &str, method: &str, does: Does, when: Option<When>) {
        let same = |a: &&mut Aim| a.target == target && a.method == method;
        match self.aims.iter_mut().find(same) {
            Some(known) => known.does = known.does.with(does),
            None => self.aims.push(Aim {
                target: target.to_string(),
                method: method.to_string(),
                does,
                when,
            }),
        }
    }

    fn finish(mut self) -> Self {
        self.aims
            .sort_by(|a, b| (&a.target, &a.method).cmp(&(&b.target, &b.method)));
        for aim in &self.aims {
            self.facts.add_aim(aim);
        }
        self
    }
}

fn assembly_folders(mod_root: &Path, game_version: &str) -> Vec<PathBuf> {
    let has_code = |folder: PathBuf| Some(folder.join("Assemblies")).filter(|f| f.is_dir());
    let loaded = crate::rimworld::loadfolders::folders(mod_root, game_version, &|_| false);
    let found: Vec<PathBuf> = loaded.into_iter().filter_map(has_code).collect();
    if !found.is_empty() {
        return found;
    }
    let inside = std::fs::read_dir(mod_root).into_iter().flatten().flatten();
    let mut older: Vec<PathBuf> = inside.filter_map(|e| has_code(e.path())).collect();
    older.sort();
    older.pop().into_iter().collect()
}

/// Only assemblies that reference `Assembly-CSharp` count: bundled libraries
/// say nothing about the mod.
pub fn read(mod_root: &Path, game_version: &str, game: &Game) -> Code {
    let mut code = Code::default();
    for dll in assembly_files(mod_root, game_version) {
        let Ok(bytes) = std::fs::read(&dll) else {
            continue;
        };
        let Some(assembly) = Assembly::read(&bytes) else {
            continue;
        };
        if assembly.references().iter().any(|r| r == "Assembly-CSharp") {
            code.add(
                &assembly.types(),
                &assembly.patches(),
                &assembly.looked_up(),
                &assembly.hand_applied(),
                game,
            );
        }
    }
    code.finish()
}

/// Every `.dll` in the mod's assembly folders for that game version.
pub fn assembly_files(mod_root: &Path, game_version: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for folder in assembly_folders(mod_root, game_version) {
        let files = std::fs::read_dir(folder).into_iter().flatten().flatten();
        found.extend(files.map(|e| e.path()).filter(|dll| {
            dll.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("dll"))
        }));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rimworld::dotnet::Method;

    fn a_type(name: &str, extends: Option<&str>, methods: &[(&str, bool)]) -> Type {
        Type {
            fields: Vec::new(),
            name: name.into(),
            extends: extends.map(String::from),
            methods: methods
                .iter()
                .map(|(name, takes_rect)| Method {
                    name: (*name).into(),
                    takes_rect: *takes_rect,
                })
                .collect(),
        }
    }

    fn game() -> Game {
        let types = [
            a_type(
                "Verse.Window",
                None,
                &[("DoWindowContents", true), ("Close", false)],
            ),
            a_type(
                "Verse.Dialog_Slider",
                Some("Verse.Window"),
                &[("Accept", false)],
            ),
            a_type("Verse.Thing", None, &[("Tick", false), ("Destroy", false)]),
            a_type("Verse.Pawn", Some("Verse.Thing"), &[("Kill", false)]),
            a_type("Verse.Mod", None, &[("DoSettingsWindowContents", true)]),
            a_type("Verse.MapComponent", None, &[("MapComponentTick", false)]),
            a_type("Verse.ModSettings", None, &[("ExposeData", false)]),
            a_type("Verse.PlayDataLoader", None, &[("LoadAll", false)]),
        ];
        let runs = [
            ("Verse.Window", "DoWindowContents", When::Interface),
            ("Verse.Thing", "Tick", When::Playing),
            ("Verse.Thing", "Destroy", When::Playing),
            ("Verse.Pawn", "Kill", When::Playing),
            ("Verse.PlayDataLoader", "LoadAll", When::Loading),
        ];
        Game::of(&types, &runs)
    }

    fn patch(target: &str, method: &str, does: Does) -> Patch {
        Patch {
            target: target.into(),
            method: method.into(),
            does,
        }
    }

    const PREFIX: Does = Does {
        prefix: true,
        postfix: false,
        transpiler: false,
        finalizer: false,
    };
    const POSTFIX: Does = Does {
        prefix: false,
        postfix: true,
        transpiler: false,
        finalizer: false,
    };

    #[test]
    fn a_mod_s_types_are_told_apart_by_what_they_extend() {
        let types = [
            a_type("My.Dialog", Some("Verse.Window"), &[]),
            a_type("My.SubDialog", Some("My.Dialog"), &[]),
            a_type("My.Painter", None, &[("Draw", true)]),
            a_type("My.Robot", Some("Verse.Pawn"), &[]),
            a_type("My.Cache", Some("Verse.MapComponent"), &[]),
            a_type(
                "My.Settings",
                Some("Verse.Mod"),
                &[("DoSettingsWindowContents", true)],
            ),
            a_type(
                "My.Options",
                Some("Verse.ModSettings"),
                &[("DoWindowContents", true)],
            ),
            a_type("My.Util", Some("System.Object"), &[("Sum", false)]),
        ];
        let mut code = Code::default();
        code.add(&types, &[], &[], &[], &game());
        let facts = code.finish().facts;
        assert_eq!((facts.types, facts.drawing, facts.adding), (8, 3, 1));
    }

    #[test]
    fn each_patched_method_is_kept_once_with_when_the_game_runs_it() {
        let patches = [
            patch("Verse.Window", "DoWindowContents", POSTFIX),
            patch("Verse.Thing", "Tick", PREFIX),
            patch("Verse.Thing", "Tick", POSTFIX),
            patch("Verse.Pawn", "Tick", POSTFIX),
            patch("Verse.PlayDataLoader", "LoadAll", POSTFIX),
            patch("Other.Mod", "Run", PREFIX),
        ];
        let looked_up = [
            ("Verse.Thing".to_string(), "Tick".to_string()),
            ("Verse.Pawn".to_string(), "Kill".to_string()),
            ("Verse.Thing".to_string(), "someField".to_string()),
            ("Other.Type".to_string(), "Tick".to_string()),
        ];
        let mut code = Code::default();
        let by_hand = [patch("Verse.Pawn", "Kill", PREFIX)];
        code.add(&[], &patches, &looked_up, &by_hand, &game());
        let code = code.finish();
        let aims: Vec<(String, Option<When>)> = code
            .aims
            .iter()
            .map(|a| (format!("{}.{}", a.target, a.method), a.when))
            .collect();
        let aim = |name: &str, when| (name.to_string(), when);
        assert_eq!(
            aims,
            [
                aim("Other.Mod.Run", None),
                aim("Verse.Pawn.Kill", Some(When::Playing)),
                aim("Verse.Pawn.Tick", Some(When::Playing)),
                aim("Verse.PlayDataLoader.LoadAll", Some(When::Loading)),
                aim("Verse.Thing.Tick", Some(When::Playing)),
                aim("Verse.Window.DoWindowContents", Some(When::Interface)),
            ]
        );
        let tick = &code.aims[4];
        assert!(tick.does.prefix && tick.does.postfix);
        assert!(
            code.aims[1].does.prefix,
            "the patch applied by hand says what it does"
        );
        let f = code.facts;
        assert_eq!(
            (
                f.patches,
                f.on_playing,
                f.on_interface,
                f.on_loading,
                f.replacing
            ),
            (6, 3, 1, 1, 3)
        );
    }

    #[test]
    fn code_is_interface_only_when_the_facts_are_plain() {
        let facts = |types, drawing, adding, patches, on_interface| Facts {
            types,
            drawing,
            adding,
            patches,
            on_interface,
            ..Facts::default()
        };
        assert!(facts(168, 39, 0, 20, 12).is_interface());
        assert!(facts(37, 6, 0, 13, 13).is_interface());
        assert!(!facts(298, 3, 216, 36, 2).is_interface());
        assert!(!facts(9, 0, 0, 4, 4).is_interface());
        assert!(!Facts::default().is_interface());
    }
}
