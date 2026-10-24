//! Catalogs: every language has exactly the keys of `en.json` with the same `{markers}`; the
//! language is chosen by `--lang` > `JLOCK_LANG`; `--json` is the same in every language.

mod common;

use common::Env;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

fn locales() -> Vec<(String, Map<String, Value>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
    let mut out: Vec<(String, Map<String, Value>)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let tag = p.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p).unwrap();
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => (tag, map),
                other => panic!("{}: not a JSON object: {other:?}", p.display()),
            }
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The translatable keys (keys starting with `_` are notes for translators).
fn keys(map: &Map<String, Value>) -> BTreeSet<String> {
    map.keys()
        .filter(|k| !k.starts_with('_'))
        .cloned()
        .collect()
}

fn markers(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = &after[..close];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.insert(name.to_string());
        }
        rest = &after[close + 1..];
    }
    out
}

fn english() -> Map<String, Value> {
    locales()
        .into_iter()
        .find(|(tag, _)| tag == "en")
        .expect("locales/en.json")
        .1
}

#[test]
fn every_catalog_has_the_keys_of_english() {
    let en = english();
    let tags: Vec<String> = locales().into_iter().map(|(t, _)| t).collect();
    assert!(tags.contains(&"pt-BR".to_string()), "{tags:?}");
    for (tag, map) in locales() {
        let missing: Vec<_> = keys(&en).difference(&keys(&map)).cloned().collect();
        let extra: Vec<_> = keys(&map).difference(&keys(&en)).cloned().collect();
        assert!(missing.is_empty(), "{tag}: missing {missing:?}");
        assert!(extra.is_empty(), "{tag}: not in en.json {extra:?}");
        for k in keys(&map) {
            assert!(map[&k].is_string(), "{tag}: {k} is not a string");
        }
    }
}

#[test]
fn every_value_keeps_the_markers_of_english() {
    let en = english();
    for (tag, map) in locales() {
        for k in keys(&en) {
            let want = markers(en[&k].as_str().unwrap());
            let got = markers(map.get(&k).and_then(Value::as_str).unwrap_or(""));
            assert_eq!(got, want, "{tag}: {k}");
        }
    }
}

#[test]
fn lang_option_and_variable_choose_the_language() {
    let mut env = Env::new("i18n-choose");
    let done = env.run(&["--lang", "pt-BR", "status", "name:x"]);
    assert_eq!(done.code, 0, "{}", done.stderr);
    assert_eq!(done.stdout.trim(), "livre name:x");

    // Anywhere on the line, and in the `--lang=` form.
    let done = env.run(&["status", "name:x", "--lang=pt-BR"]);
    assert_eq!(done.stdout.trim(), "livre name:x");
    let done = env.run(&["status", "--lang", "pt_BR.UTF-8", "name:x"]);
    assert_eq!(done.stdout.trim(), "livre name:x");

    env.set("JLOCK_LANG", "pt-BR");
    assert_eq!(env.run(&["acquire", "name:x", "--owner", "a"]).code, 0);
    let busy = env.run(&["acquire", "name:x", "--owner", "b", "--no-wait"]);
    assert_eq!(busy.code, 10);
    assert!(busy.stdout.starts_with("ocupada name:x"), "{}", busy.stdout);
    // `--lang` wins over `JLOCK_LANG`.
    let en = env.run(&["--lang", "en", "status", "name:y"]);
    assert_eq!(en.stdout.trim(), "free name:y");
    // Errors too.
    let err = env.run(&["status"]);
    assert_eq!(err.code, 2);
    assert!(
        err.stderr.contains("status precisa de uma chave"),
        "{}",
        err.stderr
    );
}

#[test]
fn unknown_language_falls_back_to_english_with_a_warning() {
    let mut env = Env::new("i18n-unknown");
    env.set("JLOCK_LANG", "xx-YY");
    let done = env.run(&["status", "name:x"]);
    assert_eq!(done.code, 0);
    assert_eq!(done.stdout.trim(), "free name:x");
    assert!(
        done.stderr.contains("unknown language 'xx-YY'"),
        "{}",
        done.stderr
    );

    let done = env.run(&["--lang", "zz", "status", "name:x"]);
    assert_eq!(done.code, 0);
    assert_eq!(done.stdout.trim(), "free name:x");
    assert!(
        done.stderr.contains("unknown language 'zz'"),
        "{}",
        done.stderr
    );
}

#[test]
fn json_is_the_same_in_every_language() {
    let env = Env::new("i18n-json");
    let cases: [&[&str]; 4] = [
        &["status", "name:x", "--json"],
        &["list", "--json"],
        &["graph", "--json"],
        &["status", "--json"],
    ];
    for args in cases {
        let en = env.run(&[&["--lang", "en"], args].concat());
        let pt = env.run(&[&["--lang", "pt-BR"], args].concat());
        assert_eq!(en.code, pt.code, "{args:?}");
        assert_eq!(en.stdout, pt.stdout, "{args:?}");
    }
    let err = env.run(&["--lang", "pt-BR", "status", "--json"]);
    assert_eq!(err.json()["error"], "status needs a key");
}
