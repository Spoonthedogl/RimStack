//! Every thread RimStack starts goes through here, named for the diagnostics.

use core::pin::Pin;
use cxx_qt::{CxxQtThread, Threading};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    let started = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(work);
    if let Err(problem) = started {
        rimstack_core::problem!("{name} not started: {problem}");
    }
}

/// `work` runs off the window's thread; an object gone by then is not told.
pub(crate) fn run<T, R>(
    name: &str,
    thread: CxxQtThread<T>,
    work: impl FnOnce() -> R + Send + 'static,
    done: impl FnOnce(Pin<&mut T>, R) + Send + 'static,
) where
    T: Threading + 'static,
    R: Send + 'static,
{
    spawn(name, move || {
        let found = work();
        if let Err(problem) = thread.queue(move |object| done(object, found)) {
            rimstack_core::problem!("a finished task found nothing to tell: {problem:?}");
        }
    });
}

const STALL: Duration = Duration::from_millis(50);

/// Under the checks only: every 20 ms; ends with the window.
pub(crate) fn watch_stalls<T: Threading + 'static>(thread: CxxQtThread<T>) {
    spawn("stall watch", move || {
        loop {
            let (ran, waiting) = std::sync::mpsc::channel();
            let asked = Instant::now();
            let queued = thread.queue(move |_| {
                let _ = ran.send(());
            });
            if queued.is_err() || waiting.recv().is_err() {
                return;
            }
            let waited = asked.elapsed();
            if waited >= STALL {
                rimstack_core::note!("stall_ms={}", waited.as_millis());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
}

#[derive(Default)]
pub(crate) struct Lane {
    asked: Arc<AtomicU64>,
}

#[derive(Clone)]
pub(crate) struct Turn {
    asked: Arc<AtomicU64>,
    mine: u64,
}

impl Turn {
    pub(crate) fn current(&self) -> bool {
        self.asked.load(Ordering::SeqCst) == self.mine
    }

    /// Waits, then says whether nothing newer was asked meanwhile: the pause of a field that applies itself.
    pub(crate) fn after(&self, wait: Duration) -> bool {
        std::thread::sleep(wait);
        self.current()
    }
}

impl Lane {
    pub(crate) fn next(&self) -> Turn {
        Turn {
            asked: self.asked.clone(),
            mine: self.asked.fetch_add(1, Ordering::SeqCst) + 1,
        }
    }

    pub(crate) fn cancel(&self) {
        self.asked.fetch_add(1, Ordering::SeqCst);
    }

    /// `done` is not called once the lane was asked again; `work` may stop early.
    pub(crate) fn run<T, R>(
        &self,
        name: &str,
        thread: CxxQtThread<T>,
        work: impl FnOnce(&Turn) -> R + Send + 'static,
        done: impl FnOnce(Pin<&mut T>, R) + Send + 'static,
    ) where
        T: Threading + 'static,
        R: Send + 'static,
    {
        let turn = self.next();
        spawn(name, move || {
            let found = work(&turn);
            // Checked on the window's thread: another run may be asked
            // for between here and there.
            let told = thread.queue(move |object| {
                if turn.current() {
                    done(object, found);
                }
            });
            if let Err(problem) = told {
                rimstack_core::problem!("a finished task found nothing to tell: {problem:?}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_newest_turn_of_a_lane_is_current() {
        let lane = Lane::default();
        let first = lane.next();
        assert!(first.current());
        let second = lane.next();
        assert!(!first.current() && second.current());
        lane.cancel();
        assert!(!second.current());
    }
}
