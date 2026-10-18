//! A part is told on its own thread, after the change is done: never while
//! the one that changed it is still running.

use rimstack_core::base::locks::Held;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changes(u32);

impl Changes {
    pub const NONE: Self = Self(0);
    /// Every `ModId` from before is void.
    pub const LIBRARY: Self = Self(1);
    pub const LOAD_ORDER: Self = Self(1 << 1);
    pub const PROBLEMS: Self = Self(1 << 2);
    pub const NOTES: Self = Self(1 << 3);
    pub const RULES: Self = Self(1 << 4);
    pub const SETTINGS: Self = Self(1 << 5);
    pub const LANGUAGE: Self = Self(1 << 6);
    pub const THEME: Self = Self(1 << 7);
    pub const COMMUNITY: Self = Self(1 << 8);
    pub const UPDATES: Self = Self(1 << 9);
    pub const TEXTURES: Self = Self(1 << 10);
    pub const FILES_READ: Self = Self(1 << 11);
    pub const LOGS: Self = Self(1 << 12);
    pub const SAVES: Self = Self(1 << 13);
    pub const LISTS: Self = Self(1 << 14);
    pub const SEARCH: Self = Self(1 << 15);
    pub const LAUNCHES: Self = Self(1 << 16);
    pub const FOLDERS: Self = Self(1 << 17);
    pub const MOD_FILES: Self = Self(1 << 18);
    pub const PATCHES: Self = Self(1 << 19);

    pub fn contains(self, any_of: Self) -> bool {
        self.0 & any_of.0 != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Changes {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for Changes {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

type Listener = Box<dyn Fn(Changes) + Send + Sync>;

/// Who is told of changes. Each listener hands the news to its own
/// object's thread (`CxxQtThread::queue`), so telling never runs inside
/// whoever published.
#[derive(Default)]
pub struct Bus {
    listeners: Mutex<Vec<Listener>>,
}

impl Bus {
    pub fn listen(&self, listener: impl Fn(Changes) + Send + Sync + 'static) {
        self.listeners.held().push(Box::new(listener));
    }

    pub fn publish(&self, changes: Changes) {
        if changes.is_empty() {
            return;
        }
        for listener in self.listeners.held().iter() {
            listener(changes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn every_listener_hears_each_change_and_nothing_is_not_told() {
        let bus = Bus::default();
        let heard = Arc::new(Mutex::new(Vec::new()));
        for _ in 0..2 {
            let heard = heard.clone();
            bus.listen(move |changes| heard.held().push(changes));
        }
        bus.publish(Changes::NONE);
        bus.publish(Changes::NOTES | Changes::PROBLEMS);
        let heard = heard.held();
        assert_eq!(heard.len(), 2);
        assert!(heard[0].contains(Changes::PROBLEMS) && !heard[0].contains(Changes::LIBRARY));
    }
}
