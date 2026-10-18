use super::{Fix, Trouble};
use crate::mods::library::Library;
use crate::mods::list::Mod;
use crate::mods::patch_order::Unfixed;
use crate::say_n;
use std::path::Path;

/// `all` is the places of `mods`, as `troubles` returns it.
pub(super) fn add(all: &mut [Vec<Trouble>], library: &Library, mods: &[Mod], unfixed: &[Unfixed]) {
    for one in unfixed {
        let folder = mods
            .get(one.mod_at)
            .and_then(|m| m.installed)
            .and_then(|id| library.get(id))
            .map(|installed| installed.path.as_path());
        if let Some(own) = all.get_mut(one.mod_at) {
            own.push(warning(folder, one));
        }
    }
}

fn warning(folder: Option<&Path>, one: &Unfixed) -> Trouble {
    Trouble {
        error: false,
        words: one.words.clone(),
        short: say_n!(one.count, "{} patch error", "{} patch errors"),
        fix: Fix::None,
        key: format!("patch:{}", relative(folder, &one.file)),
    }
}

fn relative(folder: Option<&Path>, file: &Path) -> String {
    let inside = folder
        .and_then(|folder| file.strip_prefix(folder).ok())
        .or_else(|| file.file_name().map(Path::new))
        .unwrap_or(file);
    inside
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::say;
    use std::path::PathBuf;

    fn unfixed(file: &str) -> Unfixed {
        Unfixed {
            mod_at: 0,
            file: PathBuf::from(file),
            count: 2,
            words: say!("2 errors in {}", file),
        }
    }

    #[test]
    fn an_unfixed_patch_is_a_warning_naming_the_file() {
        let one = unfixed("/mods/a/Patches/p.xml");
        let found = warning(Some(Path::new("/mods/a")), &one);
        assert!(!found.error);
        assert_eq!(found.fix, Fix::None);
        assert!(found.words.to_string().contains("Patches/p.xml"));
        assert_eq!(found.short.to_string(), "2 patch errors");
        assert_eq!(found.key, "patch:Patches/p.xml");
    }

    #[test]
    fn the_key_falls_back_to_the_file_name_outside_the_folder() {
        let one = unfixed("/elsewhere/Patches/p.xml");
        assert_eq!(warning(None, &one).key, "patch:p.xml");
        assert_eq!(warning(Some(Path::new("/mods/a")), &one).key, "patch:p.xml");
    }

    #[test]
    fn add_puts_each_on_its_mod_and_skips_a_place_out_of_range() {
        let mut all = vec![Vec::new(), Vec::new()];
        let library = Library::default();
        let mut far = unfixed("p.xml");
        far.mod_at = 5;
        let mut second = unfixed("q.xml");
        second.mod_at = 1;
        add(&mut all, &library, &[], &[unfixed("p.xml"), second, far]);
        assert_eq!(all[0].len(), 1);
        assert_eq!(all[1].len(), 1);
    }
}
