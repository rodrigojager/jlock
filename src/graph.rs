//! "Who waits for whom" (plan decision 7). Nodes are owners; each live ticket gives edges from its
//! owner to the owners of the holds that block it (`held`) and, unless its family already covers
//! the key, to the owners of older conflicting tickets (`queued`, they go first). The edges come
//! from the same rule that grants (`conflict::judge`), so shared holds and the owner's own family
//! never make an edge.
//!
//! Hierarchy: an owner waits whenever one of its descendants waits (a task waits for the commands
//! it started). So, walking the graph, an owner also has the edges of its descendants, and a chain
//! that comes back to an ancestor of the waiter it started from is a cycle too.

use crate::conflict::{conflicts, judge, owner_ancestor, Claim};
use crate::fsx;
use crate::heartbeat::stale_after;
use crate::holder::Mode;
use crate::index::KeyState;
use crate::key::Key;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// Waits for the holder of `key`.
    Held,
    /// Waits behind an older ticket in the queue of `key`.
    Queued,
}

#[derive(Clone, Debug, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub key: String,
    pub via: Via,
    /// The ticket of `from` that makes it wait.
    pub ticket: String,
    /// The mode `from` asks for.
    pub mode: Mode,
    /// The mode of the hold (or ticket) of `to` in the way.
    pub to_mode: Mode,
}

/// The edges `owner` would have when waiting for `key` in `mode` with ticket `mine` (`None`: a
/// ticket not created yet, younger than every existing one).
pub fn edges_for(
    owner: &str,
    key: &Key,
    mode: Mode,
    mine: Option<&str>,
    states: &[KeyState],
) -> Vec<Edge> {
    let ticket = mine.unwrap_or("").to_string();
    let mut holds: Vec<Claim> = Vec::new();
    let mut tickets: Vec<Claim> = Vec::new();
    for st in states.iter().filter(|st| conflicts(key, &st.dir.key)) {
        holds.extend(st.hold_claims());
        tickets.extend(st.tickets.iter().filter(|t| live(t)).map(|t| Claim {
            key: st.dir.key.clone(),
            id: t.name.clone(),
            body: t.body.clone(),
        }));
    }
    let v = judge(key, mode, owner, mine, &holds, &tickets);
    let edge = |c: &Claim, via: Via| Edge {
        from: owner.into(),
        to: c.body.owner.clone(),
        key: c.key.text.clone(),
        via,
        ticket: ticket.clone(),
        mode,
        to_mode: c.body.mode,
    };
    let mut out: Vec<Edge> = v.blockers.iter().map(|c| edge(c, Via::Held)).collect();
    out.extend(v.ahead.iter().map(|c| edge(c, Via::Queued)));
    out
}

fn live(t: &crate::queue::Ticket) -> bool {
    fsx::age(&t.path).is_ok_and(|a| a <= stale_after())
}

/// Every edge of the registry. Dead tickets (stale heartbeat) are left out: a dead waiter waits
/// for nobody.
pub fn edges(states: &[KeyState]) -> Vec<Edge> {
    let mut out = Vec::new();
    for st in states {
        for t in st.tickets.iter().filter(|t| live(t)) {
            out.extend(edges_for(
                &t.body.owner,
                &st.dir.key,
                t.body.mode,
                Some(&t.name),
                states,
            ));
        }
    }
    out
}

/// Does node `node` wait through `e` (its own edge, or one of a descendant)?
fn leaves(node: &str, e: &Edge) -> bool {
    e.from == node || owner_ancestor(node, &e.from)
}

/// Shortest chain of edges from owner `from` back to owner `to` (or to an ancestor of `to`, which
/// waits whenever `to` does).
pub fn path<'a>(edges: &'a [Edge], from: &str, to: &str) -> Option<Vec<&'a Edge>> {
    let mut seen: HashSet<&str> = HashSet::from([from]);
    let mut queue: VecDeque<(&str, Vec<&Edge>)> = VecDeque::from([(from, Vec::new())]);
    while let Some((node, trail)) = queue.pop_front() {
        for e in edges.iter().filter(|e| leaves(node, e)) {
            let mut next = trail.clone();
            next.push(e);
            if e.to == to || owner_ancestor(&e.to, to) {
                return Some(next);
            }
            if seen.insert(e.to.as_str()) {
                queue.push_back((e.to.as_str(), next));
            }
        }
    }
    None
}

/// `A → y (held by B) → B → x (held by A) → A`. A step taken through a descendant's wait shows it:
/// `→ P ⊃ P/exec-1 → ...`.
pub fn chain_text(chain: &[&Edge]) -> String {
    let Some(first) = chain.first() else {
        return String::new();
    };
    let mut out = first.from.clone();
    let mut at = first.from.as_str();
    for e in chain {
        if e.from != at {
            out += &format!(" ⊃ {}", e.from);
        }
        let how = match e.via {
            Via::Held => "held by",
            Via::Queued => "queued behind",
        };
        out += &format!(" → {} ({how} {}) → {}", e.key, e.to, e.to);
        at = e.to.as_str();
    }
    out
}

/// A cycle that starts with one of `mine` (edges of one ticket) and comes back to its owner.
pub fn cycle_through<'a>(all: &'a [Edge], mine: &[&'a Edge]) -> Option<Vec<&'a Edge>> {
    for first in mine {
        if let Some(rest) = path(all, &first.to, &first.from) {
            let mut cycle = vec![*first];
            cycle.extend(rest);
            return Some(cycle);
        }
    }
    None
}

/// All distinct cycles (as owner sets), each as a chain.
pub fn cycles(all: &[Edge]) -> Vec<Vec<&Edge>> {
    let mut seen: HashSet<Vec<String>> = HashSet::new();
    let mut out = Vec::new();
    for e in all {
        if let Some(cycle) = cycle_through(all, &[e]) {
            let mut nodes: Vec<String> = cycle.iter().map(|e| e.from.clone()).collect();
            nodes.sort();
            nodes.dedup();
            if seen.insert(nodes) {
                out.push(cycle);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(from: &str, to: &str, key: &str) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            key: key.into(),
            via: Via::Held,
            ticket: String::new(),
            mode: Mode::Exclusive,
            to_mode: Mode::Exclusive,
        }
    }

    #[test]
    fn plain_cycle() {
        let all = [e("a", "b", "y"), e("b", "a", "x")];
        let c = cycle_through(&all, &[&all[0]]).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(chain_text(&c), "a → y (held by b) → b → x (held by a) → a");
    }

    #[test]
    fn a_descendant_waiting_makes_its_ancestor_wait() {
        // The child of P waits for S; S waits for P (which waits for its child): a cycle.
        let all = [e("P/exec", "S", "y"), e("S", "P", "x")];
        let c = cycle_through(&all, &[&all[0]]).unwrap();
        assert_eq!(c.len(), 2);
        // P holds x and its child waits for S, who waits for x: seen from S too.
        let c = cycle_through(&all, &[&all[1]]).unwrap();
        assert_eq!(
            chain_text(&c),
            "S → x (held by P) → P ⊃ P/exec → y (held by S) → S"
        );
    }

    #[test]
    fn siblings_waiting_on_each_other_without_a_cycle() {
        let all = [e("r/T2/exec", "r/T1", "a")];
        assert!(cycles(&all).is_empty());
    }
}
