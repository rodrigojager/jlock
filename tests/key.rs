#[path = "../src/key.rs"]
#[allow(dead_code)]
mod key;

use key::{normalize_for, sha1_hex, Kind, Platform};

fn win(input: &str) -> String {
    normalize_for(input, r"D:\Work\Proj", r"C:\Users\Me", Platform::Windows)
        .unwrap()
        .text
}

fn linux(input: &str) -> String {
    normalize_for(input, "/home/me/Proj", "/home/me", Platform::Linux)
        .unwrap()
        .text
}

fn mac(input: &str) -> String {
    normalize_for(input, "/Users/Me/Proj", "/Users/Me", Platform::Mac)
        .unwrap()
        .text
}

#[test]
fn windows_case_and_separators() {
    assert_eq!(win(r"D:\X\A.TS"), "d:/x/a.ts");
    assert_eq!(win("d:/x/a.ts"), "d:/x/a.ts");
    assert_eq!(win(r"D:\X\A.TS"), win("d:/x/a.ts"));
    assert_eq!(win(r"d:\x\sub\"), "d:/x/sub");
    assert_eq!(win(r"d:\x\\sub\.\y\..\z"), "d:/x/sub/z");
}

#[test]
fn windows_relative_and_verbatim() {
    assert_eq!(win(r"src\a.ts"), "d:/work/proj/src/a.ts");
    assert_eq!(win(r".\src\a.ts"), "d:/work/proj/src/a.ts");
    assert_eq!(win(r"..\other"), "d:/work/other");
    assert_eq!(win(r"src\a.ts"), win(r"D:\Work\Proj\src\a.ts"));
    assert_eq!(win(r"\\?\D:\x"), "d:/x");
    assert_eq!(win(r"\\?\UNC\Server\Share\Dir"), "//server/share/dir");
    assert_eq!(win(r"\Temp\x"), "d:/temp/x");
    assert_eq!(win("~/notes.txt"), "c:/users/me/notes.txt");
    assert_eq!(win(r"D:\"), "d:/");
}

#[test]
fn linux_keeps_case() {
    assert_eq!(linux("/srv/Data/File.TXT"), "/srv/Data/File.TXT");
    assert_eq!(linux("src/A.rs"), "/home/me/Proj/src/A.rs");
    assert_eq!(linux("~"), "/home/me");
    assert_eq!(linux("~/x/"), "/home/me/x");
    assert_eq!(linux("/a//b/./c/../d/"), "/a/b/d");
    assert_eq!(linux("/"), "/");
}

#[test]
fn mac_folds_case() {
    assert_eq!(mac("/Users/Me/Src/A.rs"), "/users/me/src/a.rs");
    assert_eq!(mac("~/Docs"), "/users/me/docs");
    assert_eq!(mac("Src"), "/users/me/proj/src");
}

#[test]
fn names_and_all() {
    let k = normalize_for("  name:porta-3000 ", "/", "/", Platform::Linux).unwrap();
    assert_eq!(k.text, "name:porta-3000");
    assert_eq!(k.kind, Kind::Name);
    let k = normalize_for("name: Porta 3000 ", "/", "/", Platform::Windows).unwrap();
    assert_eq!(k.text, "name:Porta 3000");
    let k = normalize_for("*", "/", "/", Platform::Windows).unwrap();
    assert_eq!(k.text, "*");
    assert_eq!(k.kind, Kind::All);
    assert!(normalize_for("name:", "/", "/", Platform::Linux).is_err());
    assert!(normalize_for("  ", "/", "/", Platform::Linux).is_err());
}

#[test]
fn hash_is_sha1_hex_of_text() {
    let k = normalize_for(r"D:\X\A.TS", "d:/", "c:/", Platform::Windows).unwrap();
    assert_eq!(k.hash, sha1_hex("d:/x/a.ts"));
    assert_eq!(k.kind, Kind::Path);
    assert_eq!(sha1_hex("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
}

#[test]
fn status_of_free_key_creates_no_key_folder() {
    let home = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("key-status-free");
    let _ = std::fs::remove_dir_all(&home);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_jlock"))
        .args(["status", r"D:\x\a.ts"])
        .env("JLOCK_HOME", &home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let expected = if cfg!(windows) {
        "free d:/x/a.ts"
    } else {
        "free"
    };
    assert!(String::from_utf8_lossy(&out.stdout).starts_with(expected));
    let entries: Vec<_> = std::fs::read_dir(home.join("locks")).unwrap().collect();
    assert!(entries.is_empty());
    let top: Vec<_> = std::fs::read_dir(&home)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(top, vec![std::ffi::OsString::from("locks")]);
    std::fs::remove_dir_all(&home).unwrap();
}
