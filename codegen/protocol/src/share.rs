//! Decides which definitions are shared between versions.
//!
//! Two versions share the definition `(scope, name)` iff the emitted code of both is
//! identical (placeholder references compared by key) AND every definition it references
//! is shared between the two as well. This is the greatest fixpoint of a partition
//! refinement (like DFA minimisation), so recursive types are handled: start with classes
//! by text, then split classes whose members reference different classes, until stable.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Kind, VERSIONS, VersionOut};

type Key = (String, String);

pub struct Shared {
    /// Per version: key -> class.
    class: Vec<BTreeMap<Key, usize>>,
    /// Per class: (owner version = first user, users).
    classes: Vec<(usize, BTreeSet<usize>)>,
}

impl Shared {
    fn class_of(&self, vi: usize, scope: &str, name: &str) -> usize {
        *self.class[vi]
            .get(&(scope.to_string(), name.to_string()))
            .unwrap_or_else(|| panic!("{}: unknown item {scope}::{name}", VERSIONS[vi].name))
    }

    /// The version whose `defs` module holds the definition this version uses.
    pub fn owner(&self, vi: usize, scope: &str, name: &str) -> usize {
        self.classes[self.class_of(vi, scope, name)].0
    }

    /// Every version using that definition.
    pub fn users(&self, vi: usize, scope: &str, name: &str) -> &BTreeSet<usize> {
        &self.classes[self.class_of(vi, scope, name)].1
    }
}

/// Placeholder references (`⟪scope|Name⟫`) in an item's text.
pub fn refs(text: &str) -> Vec<Key> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('\u{27ea}') {
        let after = &rest[start + '\u{27ea}'.len_utf8()..];
        let end = after.find('\u{27eb}').expect("unterminated reference");
        let (scope, name) = after[..end].split_once('|').expect("reference");
        out.push((scope.to_string(), name.to_string()));
        rest = &after[end + '\u{27eb}'.len_utf8()..];
    }
    out.sort();
    out.dedup();
    out
}

pub fn analyse(outs: &[VersionOut]) -> Shared {
    // Flatten: node = (version, item index).
    let mut nodes: Vec<(usize, usize)> = Vec::new();
    let mut node_of: Vec<BTreeMap<Key, usize>> = vec![BTreeMap::new(); outs.len()];
    for (vi, out) in outs.iter().enumerate() {
        for (ii, v) in out.items.iter().enumerate() {
            node_of[vi].insert((v.item.scope.clone(), v.item.name.clone()), nodes.len());
            nodes.push((vi, ii));
        }
    }
    let edges: Vec<Vec<usize>> = nodes
        .iter()
        .map(|&(vi, ii)| {
            refs(&outs[vi].items[ii].item.text)
                .into_iter()
                .map(|k| {
                    *node_of[vi].get(&k).unwrap_or_else(|| {
                        panic!(
                            "{}: {}::{} references unknown {}::{}",
                            VERSIONS[vi].name,
                            outs[vi].items[ii].item.scope,
                            outs[vi].items[ii].item.name,
                            k.0,
                            k.1
                        )
                    })
                })
                .collect()
        })
        .collect();

    // Initial classes: by key and text.
    let mut class: Vec<usize> = {
        let mut ids: BTreeMap<(Key, &str), usize> = BTreeMap::new();
        nodes
            .iter()
            .map(|&(vi, ii)| {
                let it = &outs[vi].items[ii].item;
                let k = ((it.scope.clone(), it.name.clone()), it.text.as_str());
                let n = ids.len();
                *ids.entry(k).or_insert(n)
            })
            .collect()
    };
    let mut count = class.iter().copied().collect::<BTreeSet<_>>().len();
    loop {
        let mut ids: BTreeMap<(usize, Vec<usize>), usize> = BTreeMap::new();
        let next: Vec<usize> = (0..nodes.len())
            .map(|n| {
                let sig = (class[n], edges[n].iter().map(|&e| class[e]).collect());
                let len = ids.len();
                *ids.entry(sig).or_insert(len)
            })
            .collect();
        let c = ids.len();
        class = next;
        if c == count {
            break;
        }
        count = c;
    }

    let mut classes: Vec<(usize, BTreeSet<usize>)> = vec![(usize::MAX, BTreeSet::new()); count];
    for (n, &(vi, _)) in nodes.iter().enumerate() {
        let e = &mut classes[class[n]];
        e.0 = e.0.min(vi);
        e.1.insert(vi);
    }
    let class_map = node_of
        .iter()
        .map(|m| m.iter().map(|(k, &n)| (k.clone(), class[n])).collect())
        .collect();
    Shared {
        class: class_map,
        classes,
    }
}

/// Prints how much is shared.
pub fn report(outs: &[VersionOut], shared: &Shared) {
    for (vi, out) in outs.iter().enumerate() {
        let (mut packets, mut own_p, mut items, mut own) = (0, 0, 0, 0);
        let mut with: BTreeMap<usize, usize> = BTreeMap::new();
        for v in &out.items {
            let o = shared.owner(vi, &v.item.scope, &v.item.name);
            items += 1;
            if o == vi {
                own += 1;
            } else {
                *with.entry(o).or_default() += 1;
            }
            if matches!(v.kind, Kind::Packet { .. }) {
                packets += 1;
                if o == vi {
                    own_p += 1;
                }
            }
        }
        let from: Vec<String> = with
            .iter()
            .map(|(o, n)| format!("{n} from {}", VERSIONS[*o].name))
            .collect();
        println!(
            "{}: {items} definitions ({packets} packets), {own} own ({own_p} packets), reused: {}",
            VERSIONS[vi].name,
            if from.is_empty() {
                "none".to_string()
            } else {
                from.join(", ")
            }
        );
    }
}
