//! "Who waits for whom" (plan decision 7). Nodes are owners; each live ticket gives edges from its
//! owner to the holder of its key and of every conflicting key, and to the owners of older tickets
//! in those queues (they go first). A cycle through an owner is a deadlock.

use crate::conflict::conflicts;
use crate::fsx;
use crate::heartbeat::stale_after;
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
}

/// The edges `owner` would have when waiting for `key` with ticket `mine` (`None`: a ticket not
/// created yet, younger than every existing one).
pub fn edges_for(owner: &str, key: &Key, mine: Option<&str>, states: &[KeyState]) -> Vec<Edge> {
    let ticket = mine.unwrap_or("").to_string();
    let mut out = Vec::new();
    for st in states.iter().filter(|st| conflicts(key, &st.dir.key)) {
        if let Some(h) = st.holder.as_ref().filter(|h| h.owner != owner) {
            out.push(Edge {
                from: owner.into(),
                to: h.owner.clone(),
                key: st.dir.key.text.clone(),
                via: Via::Held,
                ticket: ticket.clone(),
            });
        }
        for t in &st.tickets {
            let older = mine.is_none_or(|m| t.name.as_str() < m);
            if older && t.body.owner != owner && live(t) {
                out.push(Edge {
                    from: owner.into(),
                    to: t.body.owner.clone(),
                    key: st.dir.key.text.clone(),
                    via: Via::Queued,
                    ticket: ticket.clone(),
                });
            }
        }
    }
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
            out.extend(edges_for(&t.body.owner, &st.dir.key, Some(&t.name), states));
        }
    }
    out
}

/// Shortest chain of edges from owner `from` to owner `to`.
pub fn path<'a>(edges: &'a [Edge], from: &str, to: &str) -> Option<Vec<&'a Edge>> {
    let mut seen: HashSet<&str> = HashSet::from([from]);
    let mut queue: VecDeque<(&str, Vec<&Edge>)> = VecDeque::from([(from, Vec::new())]);
    while let Some((node, trail)) = queue.pop_front() {
        for e in edges.iter().filter(|e| e.from == node) {
            let mut next = trail.clone();
            next.push(e);
            if e.to == to {
                return Some(next);
            }
            if seen.insert(e.to.as_str()) {
                queue.push_back((e.to.as_str(), next));
            }
        }
    }
    None
}

/// `A → y (held by B) → B → x (held by A) → A`
pub fn chain_text(chain: &[&Edge]) -> String {
    let Some(first) = chain.first() else {
        return String::new();
    };
    let mut out = first.from.clone();
    for e in chain {
        let how = match e.via {
            Via::Held => "held by",
            Via::Queued => "queued behind",
        };
        out += &format!(" → {} ({how} {}) → {}", e.key, e.to, e.to);
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
            if seen.insert(nodes) {
                out.push(cycle);
            }
        }
    }
    out
}
