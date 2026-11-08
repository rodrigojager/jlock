//! Which requests and holds exclude each other (plan section 5, revised for 0.3).
//!
//! Keys: equal keys conflict; a folder conflicts with anything inside it; `*` conflicts with
//! everything; `name:` and `resource:` keys only conflict with the identical key (and `*`).
//!
//! Modes: two holds of conflicting keys coexist when both are `shared`.
//!
//! Owners: an owner is a `/`-separated path (`gt:run/T18/exec-1`). Owners of one family (the same
//! owner, an ancestor, a descendant) never block each other. A request covered by a hold of an
//! ancestor (that hold's key is the same key, a folder above it, or `*`) is a nested hold: it is
//! granted at once, skips the queue, and lives no longer than the hold it is nested in.
//!
//! The grant rule (`judge`): a request waits while some hold blocks it, or, unless it is covered by
//! a hold of its own family, while an older ticket that conflicts with it is in the way.

use crate::holder::{Holder, Mode, Parent};
use crate::index;
use crate::key::{Key, Kind};
use crate::store::{KeyDir, Store};
use std::io;

pub fn is_ancestor(a: &str, b: &str) -> bool {
    b.len() > a.len() && b.starts_with(a) && (a.ends_with('/') || b[a.len()..].starts_with('/'))
}

/// Do the two keys exclude each other (whatever the modes)?
pub fn conflicts(a: &Key, b: &Key) -> bool {
    if a.text == b.text {
        return true;
    }
    match (a.kind, b.kind) {
        (Kind::All, _) | (_, Kind::All) => true,
        (Kind::Path, Kind::Path) => is_ancestor(&a.text, &b.text) || is_ancestor(&b.text, &a.text),
        _ => false,
    }
}

/// Does a hold on `held` cover `key` (the same key, a folder above it, or `*`)?
pub fn covers(held: &Key, key: &Key) -> bool {
    held.text == key.text
        || held.kind == Kind::All
        || (held.kind == Kind::Path && key.kind == Kind::Path && is_ancestor(&held.text, &key.text))
}

/// Is owner `a` a strict ancestor of owner `b` (`gt:r` of `gt:r/T1/exec-1`)?
pub fn owner_ancestor(a: &str, b: &str) -> bool {
    b.len() > a.len() && b.starts_with(a) && b[a.len()..].starts_with('/')
}

/// The same owner, or one is an ancestor of the other: they never block each other.
pub fn related(a: &str, b: &str) -> bool {
    a == b || owner_ancestor(a, b) || owner_ancestor(b, a)
}

/// Does a request (or hold) of `a_owner` on `a_key` in `a_mode` exclude the hold (or ticket) `b`
/// on `b_key`?
pub fn excludes(a_key: &Key, a_mode: Mode, a_owner: &str, b_key: &Key, b: &Holder) -> bool {
    conflicts(a_key, b_key) && !a_mode.compatible(b.mode) && !related(a_owner, &b.owner)
}

/// A hold or a ticket seen on disk: its key, its grant or ticket name, and its body.
#[derive(Clone, Debug)]
pub struct Claim {
    pub key: Key,
    /// The grant (`holds/<grant>.json`) or ticket (`queue/<ticket>.json`) name.
    pub id: String,
    pub body: Holder,
}

/// What stands between a request and its grant.
#[derive(Clone, Debug, Default)]
pub struct Verdict {
    /// Holds that block the request.
    pub blockers: Vec<Claim>,
    /// Older tickets in the way (always empty when the request is covered by its family).
    pub ahead: Vec<Claim>,
    /// The ancestor's hold the request would be nested in.
    pub parent: Option<Parent>,
    /// Covered by a hold of the owner itself or of an ancestor: the queue does not apply.
    pub covered: bool,
}

impl Verdict {
    pub fn free(&self) -> bool {
        self.blockers.is_empty() && self.ahead.is_empty()
    }
}

/// Judges a request for `key` in `mode` by `owner`, holding ticket `mine` (`None`: no ticket yet,
/// younger than every ticket), against the holds and tickets of that key and of the conflicting
/// keys. Pure: the caller reads the disk.
pub fn judge(
    key: &Key,
    mode: Mode,
    owner: &str,
    mine: Option<&str>,
    holds: &[Claim],
    tickets: &[Claim],
) -> Verdict {
    let mut v = Verdict::default();
    // The nearest ancestor (longest owner path) whose hold covers the key; the oldest grant on a tie.
    let mut best: Option<&Claim> = None;
    for h in holds {
        if !covers(&h.key, key) {
            continue;
        }
        if h.body.owner == owner {
            v.covered = true;
        } else if owner_ancestor(&h.body.owner, owner) {
            v.covered = true;
            let nearer = best.is_none_or(|b| {
                h.body.owner.len() > b.body.owner.len()
                    || (h.body.owner.len() == b.body.owner.len() && h.id < b.id)
            });
            if nearer {
                best = Some(h);
            }
        }
    }
    v.parent = best.map(|h| Parent {
        owner: h.body.owner.clone(),
        key: h.key.text.clone(),
        grant: h.id.clone(),
    });
    v.blockers = holds
        .iter()
        .filter(|h| excludes(key, mode, owner, &h.key, &h.body))
        .cloned()
        .collect();
    if !v.covered {
        v.ahead = tickets
            .iter()
            .filter(|t| mine.is_none_or(|m| t.id.as_str() < m))
            .filter(|t| excludes(key, mode, owner, &t.key, &t.body))
            .cloned()
            .collect();
    }
    v
}

/// The other key folders whose keys conflict with `key`.
pub fn conflicting_dirs(store: &Store, key: &Key) -> io::Result<Vec<KeyDir>> {
    if matches!(key.kind, Kind::Name | Kind::Resource) {
        // Only `*` can conflict with a name or a resource: no need to read every folder.
        let all = store.key_dir(&Key::from_text("*"));
        return Ok(if all.exists() { vec![all] } else { Vec::new() });
    }
    Ok(index::dirs(store)?
        .into_iter()
        .filter(|d| d.key.text != key.text && conflicts(key, &d.key))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holder::Ident;
    use Mode::{Exclusive as X, Shared as S};

    fn k(t: &str) -> Key {
        Key::from_text(t)
    }

    fn claim(key: &str, id: &str, owner: &str, mode: Mode) -> Claim {
        let ident = Ident {
            owner: owner.into(),
            ..Ident::default()
        };
        Claim {
            key: k(key),
            id: id.into(),
            body: Holder::new(key, &ident, mode),
        }
    }

    #[test]
    fn hierarchy() {
        assert!(conflicts(&k("d:/p/src"), &k("d:/p/src/a.ts")));
        assert!(conflicts(&k("d:/p/src/a.ts"), &k("d:/p/src")));
        assert!(!conflicts(&k("d:/p/src"), &k("d:/p/docs")));
        assert!(!conflicts(&k("d:/p/src"), &k("d:/p/srcx")));
        assert!(conflicts(&k("d:/"), &k("d:/p")));
        assert!(conflicts(&k("/"), &k("/etc")));
        assert!(conflicts(&k("name:x"), &k("name:x")));
        assert!(!conflicts(&k("name:x"), &k("name:xy")));
        assert!(!conflicts(&k("name:d:/p"), &k("d:/p")));
        assert!(conflicts(&k("*"), &k("name:x")));
        assert!(conflicts(&k("d:/p"), &k("*")));
    }

    #[test]
    fn resources_only_meet_the_same_resource() {
        assert!(conflicts(&k("resource:desktop"), &k("resource:desktop")));
        assert!(!conflicts(
            &k("resource:port:8080"),
            &k("resource:port:8081")
        ));
        assert!(!conflicts(&k("resource:port"), &k("resource:port:8080")));
        assert!(!conflicts(
            &k("resource:desktop"),
            &k("name:resource:desktop")
        ));
        assert!(!conflicts(&k("resource:desktop"), &k("name:desktop")));
        assert!(!conflicts(&k("resource:desktop"), &k("/resource:desktop")));
        assert!(!conflicts(&k("resource:machine"), &k("d:/p")));
        assert!(conflicts(&k("*"), &k("resource:machine")));
    }

    #[test]
    fn owner_paths() {
        assert!(owner_ancestor("gt:r/T1", "gt:r/T1/exec-1"));
        assert!(owner_ancestor("gt:r", "gt:r/T1/exec-1"));
        assert!(!owner_ancestor("gt:r/T1", "gt:r/T1"));
        assert!(!owner_ancestor("gt:r/T1", "gt:r/T10"));
        assert!(!owner_ancestor("gt:r/T1/exec-1", "gt:r/T1"));
        assert!(related("a/b", "a/b"));
        assert!(related("a/b", "a"));
        assert!(!related("a/b", "a/c"));
    }

    #[test]
    fn modes() {
        let file = claim("d:/p/src/a.ts", "g1", "a", S);
        assert!(!excludes(
            &k("d:/p/src/a.ts"),
            S,
            "b",
            &file.key,
            &file.body
        ));
        assert!(excludes(&k("d:/p/src/a.ts"), X, "b", &file.key, &file.body));
        assert!(!excludes(&k("d:/p/src"), S, "b", &file.key, &file.body));
        // A shared folder and an exclusive file inside it conflict across modes.
        let mut excl = file.body.clone();
        excl.mode = X;
        assert!(excludes(&k("d:/p/src"), S, "b", &file.key, &excl));
    }

    #[test]
    fn same_owner_is_covered_and_never_blocked() {
        let holds = [claim("name:x", "g1", "a", X)];
        let v = judge(&k("name:x"), X, "a", None, &holds, &[]);
        assert!(v.free() && v.covered && v.parent.is_none());
    }

    #[test]
    fn ancestor_hold_nests_the_child_and_skips_the_queue() {
        // The jtasks deadlock: the task holds `*`, its exec child asks for a file under it while a
        // sibling's exec already waits for that file.
        let holds = [claim("*", "g1", "gt:r/T1", X)];
        let tickets = [claim("d:/p/a.ts", "t1", "gt:r/T2/exec-1", X)];
        let v = judge(&k("d:/p/a.ts"), X, "gt:r/T1/exec-1", None, &holds, &tickets);
        assert!(v.free(), "{v:?}");
        let parent = v.parent.unwrap();
        assert_eq!(
            (
                parent.owner.as_str(),
                parent.key.as_str(),
                parent.grant.as_str()
            ),
            ("gt:r/T1", "*", "g1")
        );
        // The sibling waits for the task's hold.
        let v = judge(
            &k("d:/p/a.ts"),
            X,
            "gt:r/T2/exec-1",
            Some("t1"),
            &holds,
            &[],
        );
        assert_eq!(v.blockers.len(), 1);
        assert_eq!(v.blockers[0].body.owner, "gt:r/T1");
    }

    #[test]
    fn nearest_ancestor_is_the_parent() {
        let holds = [
            claim("d:/p", "g1", "r", X),
            claim("d:/p/src", "g2", "r/T1", X),
        ];
        let v = judge(&k("d:/p/src/a.ts"), X, "r/T1/exec", None, &holds, &[]);
        assert!(v.free());
        assert_eq!(v.parent.unwrap().owner, "r/T1");
    }

    #[test]
    fn ancestor_with_only_descendants_in_the_way_is_granted() {
        let holds = [claim("d:/p/src/a.ts", "g1", "r/T1/exec", X)];
        let v = judge(&k("d:/p/src"), X, "r/T1", None, &holds, &[]);
        assert!(v.free() && v.parent.is_none() && !v.covered);
        // ...but not while an unrelated owner holds something inside too.
        let holds = [
            claim("d:/p/src/a.ts", "g1", "r/T1/exec", X),
            claim("d:/p/src/b.ts", "g2", "r/T2", S),
        ];
        let v = judge(&k("d:/p/src"), X, "r/T1", None, &holds, &[]);
        assert_eq!(v.blockers.len(), 1);
        assert_eq!(v.blockers[0].body.owner, "r/T2");
    }

    #[test]
    fn covered_child_still_waits_for_an_unrelated_conflicting_hold() {
        let holds = [claim("d:/p", "g1", "r/T1", S), claim("d:/p", "g2", "q", S)];
        let v = judge(&k("d:/p/a.ts"), X, "r/T1/exec", None, &holds, &[]);
        assert!(v.covered);
        assert_eq!(v.blockers.len(), 1);
        assert_eq!(v.blockers[0].body.owner, "q");
    }

    #[test]
    fn shared_requests_coexist_and_writers_are_not_starved() {
        let holds = [claim("name:db", "g1", "r1", S)];
        assert!(judge(&k("name:db"), S, "r2", None, &holds, &[]).free());
        assert_eq!(
            judge(&k("name:db"), X, "w", None, &holds, &[])
                .blockers
                .len(),
            1
        );
        // An exclusive waiter blocks later shared requests...
        let tickets = [claim("name:db", "t1", "w", X)];
        let v = judge(&k("name:db"), S, "r3", None, &holds, &tickets);
        assert!(v.blockers.is_empty());
        assert_eq!(v.ahead.len(), 1);
        // ...while an older shared waiter does not block a newer shared one.
        let tickets = [claim("name:db", "t1", "r4", S)];
        assert!(judge(&k("name:db"), S, "r5", Some("t2"), &[], &tickets).free());
        // And the exclusive waiter is not held back by younger tickets.
        let tickets = [claim("name:db", "t2", "r3", S)];
        assert!(judge(&k("name:db"), X, "w", Some("t1"), &[], &tickets).free());
    }

    #[test]
    fn resource_path_and_name_never_meet() {
        let holds = [
            claim("resource:desktop", "g1", "a", X),
            claim("name:desktop", "g2", "b", X),
            claim("d:/desktop", "g3", "c", X),
        ];
        for (key, owner) in [
            ("resource:desktop", "a"),
            ("name:desktop", "b"),
            ("d:/desktop", "c"),
        ] {
            let v = judge(&k(key), X, "z", None, &holds, &[]);
            assert_eq!(v.blockers.len(), 1, "{key}");
            assert_eq!(v.blockers[0].body.owner, owner, "{key}");
        }
    }
}
