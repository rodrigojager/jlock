//! Which keys exclude each other (plan section 5): equal keys; a folder and anything inside it;
//! `*` and everything. `name:` keys only conflict with the same name (and `*`).

use crate::index;
use crate::key::{Key, Kind};
use crate::store::{KeyDir, Store};
use std::io;

pub fn is_ancestor(a: &str, b: &str) -> bool {
    b.len() > a.len() && b.starts_with(a) && (a.ends_with('/') || b[a.len()..].starts_with('/'))
}

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

/// The other key folders whose keys conflict with `key`.
pub fn conflicting_dirs(store: &Store, key: &Key) -> io::Result<Vec<KeyDir>> {
    if key.kind == Kind::Name {
        // Only `*` can conflict with a name: no need to read every folder.
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

    fn k(t: &str) -> Key {
        Key::from_text(t)
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
}
