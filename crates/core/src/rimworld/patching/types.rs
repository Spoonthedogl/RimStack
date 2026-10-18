use crate::rimworld::dotnet::{Assembly, Field, FieldType, Type};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct Types {
    known: HashMap<String, Known>,
}

#[derive(Debug)]
struct Known {
    extends: Option<String>,
    fields: Vec<Field>,
}

/// A type's name, its base and its fields.
#[cfg(test)]
pub type TestType<'a> = (&'a str, Option<&'a str>, &'a [(&'a str, FieldType)]);

/// How far up the bases a walk goes: a damaged file may make them loop.
const DEPTH: usize = 64;

impl Types {
    /// An assembly that cannot be read is left out; a type found twice keeps its first.
    pub fn read(dlls: &[PathBuf]) -> Types {
        let mut known = HashMap::new();
        for dll in dlls {
            let Ok(bytes) = std::fs::read(dll) else {
                continue;
            };
            let Some(assembly) = Assembly::read(&bytes) else {
                continue;
            };
            for Type {
                name,
                extends,
                fields,
                ..
            } in assembly.types()
            {
                known.entry(name).or_insert(Known { extends, fields });
            }
        }
        Types { known }
    }

    #[cfg(test)]
    pub fn from_fields(types: &[TestType]) -> Types {
        let known = types
            .iter()
            .map(|(name, extends, fields)| {
                let fields = fields
                    .iter()
                    .map(|(name, of)| Field {
                        name: name.to_string(),
                        of: of.clone(),
                    })
                    .collect();
                let extends = extends.map(str::to_string);
                (name.to_string(), Known { extends, fields })
            })
            .collect();
        Types { known }
    }

    /// A `Class` attribute wins; its name may leave out `Verse.` or `RimWorld.`.
    pub fn def_type<'a>(&'a self, element: &'a str, class: Option<&'a str>) -> Option<&'a str> {
        let in_game = |name: &str| {
            ["Verse", "RimWorld"].into_iter().find_map(|namespace| {
                let full = format!("{namespace}.{name}");
                self.known.get_key_value(&full).map(|(k, _)| k.as_str())
            })
        };
        match class {
            Some(class) if self.known.contains_key(class) => Some(class),
            Some(class) => in_game(class).or(Some(class)),
            None => in_game(element),
        }
    }

    /// The type's own fields first, then each base type's.
    pub fn field<'a>(&'a self, of: &'a str, name: &str) -> Option<&'a FieldType> {
        self.bases(of)
            .find_map(|t| t.fields.iter().find(|f| f.name == name))
            .map(|f| &f.of)
    }

    pub fn is_def(&self, type_name: &str) -> bool {
        self.names(type_name).any(|name| name == "Verse.Def")
    }

    fn bases<'a>(&'a self, of: &'a str) -> impl Iterator<Item = &'a Known> {
        self.names(of).filter_map(|name| self.known.get(name))
    }

    fn names<'a>(&'a self, of: &'a str) -> impl Iterator<Item = &'a str> {
        let mut next = Some(of);
        std::iter::from_fn(move || {
            let at = next?;
            next = self.known.get(at).and_then(|t| t.extends.as_deref());
            Some(at)
        })
        .take(DEPTH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rimworld::dotnet::build::Builder;

    fn read(b: Builder) -> Types {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Mod.dll");
        std::fs::write(&path, b.finish()).unwrap();
        Types::read(&[path])
    }

    #[test]
    fn a_field_is_found_on_a_base_type() {
        let mut b = Builder::new();
        let string = [0x06, 0x0e];
        let object = b.type_ref("System", "Object");
        let def = b.type_def("Verse", "Def", object);
        b.field_of("defName", 0x06, &string);
        b.type_def_under("RimWorld", "GeneDef", def);
        b.field_of("label", 0x06, &string);
        let types = read(b);
        assert_eq!(
            types.field("RimWorld.GeneDef", "defName"),
            Some(&FieldType::Other)
        );
        assert_eq!(
            types.field("RimWorld.GeneDef", "label"),
            Some(&FieldType::Other)
        );
        assert_eq!(types.field("RimWorld.GeneDef", "missing"), None);
        assert_eq!(types.field("No.Such", "defName"), None);
        assert!(types.is_def("RimWorld.GeneDef"));
        assert!(!types.is_def("System.Object"));
        assert!(!types.is_def("No.Such"));
    }

    #[test]
    fn a_def_element_is_looked_up_in_verse_and_rimworld() {
        let mut b = Builder::new();
        b.type_def("Verse", "ThingDef", 0);
        b.type_def("RimWorld", "GeneDef", 0);
        b.type_def("Mod.Space", "Special", 0);
        let types = read(b);
        assert_eq!(types.def_type("ThingDef", None), Some("Verse.ThingDef"));
        assert_eq!(types.def_type("GeneDef", None), Some("RimWorld.GeneDef"));
        assert_eq!(types.def_type("Nothing", None), None);
        assert_eq!(
            types.def_type("ThingDef", Some("Mod.Space.Special")),
            Some("Mod.Space.Special")
        );
        assert_eq!(
            types.def_type("ThingDef", Some("GeneDef")),
            Some("RimWorld.GeneDef")
        );
    }

    /// Needs the game: `RIMSTACK_GAME`, else the usual Steam folder; skipped without one.
    #[test]
    #[ignore = "needs an installed RimWorld"]
    fn genes_is_a_list_of_defs_in_the_real_game() {
        let from_env = std::env::var_os("RIMSTACK_GAME").map(PathBuf::from);
        let usual = PathBuf::from("/home/jupiter/Games/Games/steamapps/common/RimWorld");
        let Some(dll) = [from_env, Some(usual)]
            .into_iter()
            .flatten()
            .find_map(|dir| crate::rimworld::game::Game::assembly_path(&dir))
        else {
            eprintln!("no RimWorld install found: skipped");
            return;
        };
        let types = Types::read(&[dll]);
        assert_eq!(
            types.field("RimWorld.XenotypeDef", "genes"),
            Some(&FieldType::ListOf("Verse.GeneDef".into()))
        );
        assert!(types.is_def("Verse.GeneDef"));
    }
}
