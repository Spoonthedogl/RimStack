use crate::base::sentence::Sentence;
use crate::mods::infer::Inferred;
use crate::mods::rules::{Kind, Rule};
use crate::mods::sort::{Sorted, reach};
use crate::rimworld::patching::{DefKey, Failure, Loaded, Run};
use crate::{say, say_n};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

/// Errors in one file of one mod that no load order removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unfixed {
    pub mod_at: usize,
    pub file: PathBuf,
    pub count: usize,
    pub words: Sentence,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub hints: Vec<Inferred>,
    pub unfixed: Vec<Unfixed>,
}

/// A swap that removed `failures` (indexes into the first run's) when replayed.
struct Swap {
    rule: Rule,
    failures: Vec<usize>,
}

/// `order` and every place in `rules` are places in the list. `resort` is Sort with `rules` and the
/// extra rules it is given.
pub fn decide(
    loaded: &Loaded,
    order: &[usize],
    rules: &[Rule],
    resort: &dyn Fn(&[Rule]) -> Sorted,
    names: &[String],
) -> Found {
    let first = loaded.run(order, None);
    let firm: Vec<Rule> = rules
        .iter()
        .filter(|r| r.kind.strength() > Kind::Inferred.strength())
        .copied()
        .collect();
    let reaches = reach(names.len(), &firm);
    let footprints = Footprints::of(&first);
    let mut swaps: Vec<Swap> = Vec::new();
    let mut stuck: Vec<usize> = Vec::new();
    for (at, failure) in first.failures.iter().enumerate() {
        match fixing_swap(loaded, order, failure, &first, &footprints, &reaches) {
            Some((before, after)) => {
                let rule = Rule {
                    first: before,
                    then: after,
                    kind: Kind::Patches,
                };
                match swaps.iter_mut().find(|s| s.rule == rule) {
                    Some(same) => same.failures.push(at),
                    None => swaps.push(Swap {
                        rule,
                        failures: vec![at],
                    }),
                }
            }
            None => stuck.push(at),
        }
    }

    let brings_new = |full: &Run| {
        full.failures.iter().any(|f| {
            let count = |run: &Run| run.failures.iter().filter(|g| *g == f).count();
            count(full) > count(&first)
        })
    };
    let rules_of = |swaps: &[Swap]| swaps.iter().map(|s| s.rule).collect::<Vec<_>>();
    loop {
        let sorted = resort(&rules_of(&swaps));
        swaps.retain(|swap| {
            let dropped = sorted.dropped.contains(&swap.rule);
            if dropped {
                stuck.extend(&swap.failures);
            }
            !dropped
        });
        if swaps.is_empty() {
            break;
        }
        let full = loaded.run(&sorted.order, None);
        let kept = swaps.len();
        swaps.retain_mut(|swap| {
            let (left, fixed): (Vec<usize>, Vec<usize>) = swap
                .failures
                .iter()
                .partition(|&&at| full.failures.contains(&first.failures[at]));
            stuck.extend(&left);
            swap.failures = fixed;
            !swap.failures.is_empty()
        });
        if swaps.len() < kept {
            continue;
        }
        if !brings_new(&full) {
            break;
        }
        let (mut low, mut high) = (1, swaps.len());
        while low < high {
            let middle = (low + high) / 2;
            if brings_new(&loaded.run(&resort(&rules_of(&swaps[..middle])).order, None)) {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        stuck.extend(&swaps.remove(low - 1).failures);
    }

    let name = |at: usize| names.get(at).cloned().unwrap_or_default();
    let hints = swaps
        .iter()
        .map(|swap| Inferred {
            first: swap.rule.first,
            then: swap.rule.then,
            why: why(
                &first.failures[swap.failures[0]],
                &name(swap.rule.first),
                &name(swap.rule.then),
            ),
            code: false,
        })
        .collect();
    stuck.sort_unstable();
    stuck.dedup();
    Found {
        hints,
        unfixed: unfixed(&first, &stuck),
    }
}

/// What each mod read and wrote over all its operations in the first run.
struct Footprints(HashMap<usize, (BTreeSet<DefKey>, BTreeSet<DefKey>)>);

impl Footprints {
    fn of(run: &Run) -> Footprints {
        let mut each: HashMap<usize, (BTreeSet<DefKey>, BTreeSet<DefKey>)> = HashMap::new();
        for trace in &run.traces {
            let (read, wrote) = each.entry(trace.mod_at).or_default();
            read.extend(trace.read.iter().cloned());
            wrote.extend(trace.wrote.iter().cloned());
        }
        Footprints(each)
    }

    /// One changes what the other reads. When neither does, their order cannot matter to either.
    fn interact(&self, a: usize, b: usize) -> bool {
        let (Some((read_a, wrote_a)), Some((read_b, wrote_b))) = (self.0.get(&a), self.0.get(&b))
        else {
            return false;
        };
        !wrote_a.is_disjoint(read_b) || !wrote_b.is_disjoint(read_a)
    }
}

/// The swap `(before, after)` that removes `failure`, if some pair of the mods that shaped its defs does.
/// A pair whose order cannot change what `failure` sees is not replayed, unless an unknown operation
/// leaves the traces incomplete.
fn fixing_swap(
    loaded: &Loaded,
    order: &[usize],
    failure: &Failure,
    run: &Run,
    footprints: &Footprints,
    reaches: &dyn Fn(usize, usize) -> bool,
) -> Option<(usize, usize)> {
    let defs = defs_of(failure, run);
    if defs.is_empty() {
        return None;
    }
    let mut candidates: BTreeSet<usize> = run
        .traces
        .iter()
        .filter(|t| !defs.is_disjoint(&t.read) || !defs.is_disjoint(&t.wrote))
        .map(|t| t.mod_at)
        .collect();
    if let Failure::Failed { mod_at, .. } = failure {
        candidates.insert(*mod_at);
    }
    let at: HashMap<usize, usize> = order.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let mut places: Vec<usize> = candidates
        .iter()
        .filter_map(|m| at.get(m).copied())
        .collect();
    places.sort_unstable();
    let mut pairs: Vec<(usize, usize)> = places
        .iter()
        .enumerate()
        .flat_map(|(i, &x)| places[i + 1..].iter().map(move |&y| (x, y)))
        .collect();
    pairs.sort_by_key(|&(x, y)| y - x);
    let traced = !defs.iter().any(|def| run.is_unknown(def))
        && match failure {
            Failure::Failed { target, .. } => !run.is_unknown_target(target),
            _ => true,
        };
    let failing = match failure {
        Failure::Failed { mod_at, .. } => at.get(mod_at).copied(),
        _ => None,
    };
    for (x, y) in pairs {
        let (a, b) = (order[x], order[y]);
        if reaches(a, b) {
            continue;
        }
        if traced {
            let matters = match failing {
                Some(p) if x > p => false,
                Some(p) if y >= p => true,
                _ => footprints.interact(a, b),
            };
            if !matters {
                continue;
            }
        }
        let mut swapped = order.to_vec();
        swapped.remove(x);
        swapped.insert(y, a);
        if !loaded.run(&swapped, Some(&defs)).failures.contains(failure) {
            return Some((b, a));
        }
    }
    None
}

fn defs_of(failure: &Failure, run: &Run) -> BTreeSet<DefKey> {
    match failure {
        Failure::Duplicate { def, .. } | Failure::Twice { def, .. } => {
            BTreeSet::from([def.clone()])
        }
        Failure::Failed { target, .. } => run
            .traces
            .iter()
            .flat_map(|t| &t.wrote)
            .filter(|def| run.reach(target).covers(def))
            .cloned()
            .collect(),
    }
}

/// None for an error an unknown operation could be behind, and for a repeat no patch took part in.
fn culprit(failure: &Failure, run: &Run) -> Option<(usize, PathBuf)> {
    match failure {
        Failure::Failed {
            mod_at,
            file,
            target,
            ..
        } => (!run.is_unknown_target(target)).then(|| (*mod_at, file.clone())),
        Failure::Duplicate { def, .. } | Failure::Twice { def, .. } => {
            if run.is_unknown(def) {
                return None;
            }
            let writer = run.traces.iter().rfind(|t| t.wrote.contains(def))?;
            Some((writer.mod_at, writer.file.clone()))
        }
    }
}

fn unfixed(run: &Run, stuck: &[usize]) -> Vec<Unfixed> {
    let mut grouped: BTreeMap<(usize, PathBuf), (usize, &Failure)> = BTreeMap::new();
    for &at in stuck {
        let failure = &run.failures[at];
        if let Some(place) = culprit(failure, run) {
            grouped.entry(place).or_insert((0, failure)).0 += 1;
        }
    }
    grouped
        .into_iter()
        .map(|((mod_at, file), (count, failure))| {
            let shown = file.display();
            let words = say_n!(
                count,
                "{} error in {}, first {}",
                "{} errors in {}, first {}",
                shown,
                what(failure).inside()
            );
            Unfixed {
                mod_at,
                file,
                count,
                words,
            }
        })
        .collect()
}

fn what(failure: &Failure) -> Sentence {
    match failure {
        Failure::Failed {
            operation, xpath, ..
        } => say!("{} on {}", operation, xpath),
        Failure::Duplicate { def, element } => say!("{} twice in {}", element, def.name),
        Failure::Twice { def, field, entry } => {
            say!("{} twice in {} of {}", entry, field, def.name)
        }
    }
}

fn why(failure: &Failure, before: &str, after: &str) -> Sentence {
    match failure {
        Failure::Failed { xpath, .. } => {
            say!(
                "With {} loaded before {}, the patch on {} finds its target",
                before,
                after,
                xpath
            )
        }
        Failure::Duplicate { def, element } => {
            say!(
                "With {} loaded before {}, {} is written once in {}",
                before,
                after,
                element,
                def.name
            )
        }
        Failure::Twice { def, field, entry } => say!(
            "With {} loaded before {}, {} is listed once in {} of {}",
            before,
            after,
            entry,
            field,
            def.name
        ),
    }
}

#[cfg(test)]
mod tests;
