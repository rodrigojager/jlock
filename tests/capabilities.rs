//! `jlock capabilities --json`: a stable answer with the version, the contract numbers and the
//! custody scope; the same in every language.

mod common;

use common::Env;

#[test]
fn capabilities_answer_the_contract() {
    let env = Env::new("capabilities");
    let done = env.run(&["capabilities", "--json"]);
    assert_eq!(done.code, 0, "{}", done.stderr);
    let v = done.json();
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    let caps = &v["capabilities"];
    assert!(caps["custody"].is_u64(), "{v}");
    assert!(caps["processInspect"].is_u64(), "{v}");
    if cfg!(windows) {
        assert_eq!(caps["custody"], 1);
        assert_eq!(caps["processInspect"], 1);
        assert_eq!(v["custodyScope"], "job");
    } else {
        assert_eq!(caps["custody"], 0);
        assert_eq!(caps["processInspect"], 0);
        assert_eq!(v["custodyScope"], "none");
    }
    let pt = env.run(&["--lang", "pt-BR", "capabilities", "--json"]);
    assert_eq!(
        pt.stdout, done.stdout,
        "--json is the same in every language"
    );
}

#[test]
fn capabilities_text_and_usage() {
    let env = Env::new("capabilities-text");
    let done = env.run(&["capabilities"]);
    assert_eq!(done.code, 0);
    assert!(done.stdout.contains("processInspect: "), "{}", done.stdout);
    assert!(done.stdout.contains(env!("CARGO_PKG_VERSION")));
    let extra = env.run(&["capabilities", "extra"]);
    assert_eq!(extra.code, 2);
    assert!(
        extra.stderr.contains("takes no arguments"),
        "{}",
        extra.stderr
    );
}
