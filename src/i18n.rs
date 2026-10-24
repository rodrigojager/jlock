//! Text in the user's language (plan section 14): catalogs embedded as JSON, chosen by `--lang` >
//! `JLOCK_LANG` > the system's language > English. English is the reference and the fallback for any
//! missing key. Only human text goes through here: `--json`, `log.jsonl`, `index.json`, exit codes,
//! command and option names stay the same in every language.

use serde_json::{Map, Value};
use std::fmt::Display;
use std::sync::OnceLock;

/// Every catalog, by tag. English first: it is the reference.
pub const CATALOGS: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.json")),
    ("pt-BR", include_str!("../locales/pt-BR.json")),
];

static LANG: OnceLock<&'static str> = OnceLock::new();

/// The language of this run (`en` until [`select`] ran).
pub fn lang() -> &'static str {
    LANG.get().copied().unwrap_or("en")
}

pub fn tags() -> Vec<&'static str> {
    CATALOGS.iter().map(|(tag, _)| *tag).collect()
}

/// `pt_BR.UTF-8`, `pt-br`, `PT` → the catalog tag that serves it: exact tag first, then the
/// primary language (`pt` → `pt-BR`, `zh` → `zh-CN`). `None` if no catalog fits.
pub fn resolve(input: &str) -> Option<&'static str> {
    let tag = input.split(['.', '@']).next().unwrap_or("").trim();
    let tag = tag.replace('_', "-");
    if tag.is_empty() {
        return None;
    }
    let primary = |t: &str| t.split('-').next().unwrap_or("").to_ascii_lowercase();
    CATALOGS
        .iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(&tag))
        .or_else(|| CATALOGS.iter().find(|(t, _)| primary(t) == primary(&tag)))
        .map(|(t, _)| *t)
}

#[cfg(windows)]
fn system_locale() -> Option<String> {
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
                              // SAFETY: the buffer and its length in UTF-16 units are passed together.
    let n = unsafe {
        windows_sys::Win32::Globalization::GetUserDefaultLocaleName(buf.as_mut_ptr(), 85)
    };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
}

#[cfg(unix)]
fn system_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.trim().is_empty())
        .filter(|v| v != "C" && v != "POSIX" && !v.starts_with("C."))
}

/// Removes `--lang <tag>` / `--lang=<tag>` from the arguments (never after a `--`, which belongs
/// to the command of `run`) and fixes the language of this run. An unknown tag given by the user
/// is a warning, in English, and the run goes on in English.
pub fn select(args: &mut Vec<String>) {
    let mut asked: Option<(String, &str)> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            break;
        }
        if let Some(v) = a.strip_prefix("--lang=") {
            asked = Some((v.to_string(), "--lang"));
            args.remove(i);
        } else if a == "--lang" {
            args.remove(i);
            if i < args.len() {
                asked = Some((args.remove(i), "--lang"));
            } else {
                eprintln!("jlock: --lang needs a value (one of {})", tags().join(", "));
            }
        } else {
            i += 1;
        }
    }
    if asked.is_none() {
        asked = std::env::var("JLOCK_LANG")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|v| (v, "JLOCK_LANG"));
    }
    let chosen = match asked {
        Some((tag, from)) => resolve(&tag).unwrap_or_else(|| {
            eprintln!(
                "jlock: unknown language '{tag}' in {from} (known: {}); using English",
                tags().join(", ")
            );
            "en"
        }),
        None => system_locale().and_then(|l| resolve(&l)).unwrap_or("en"),
    };
    let _ = LANG.set(chosen);
}

/// A parsed catalog. Keys starting with `_` (such as `_glossary`) are notes for translators.
pub fn parse(source: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(source) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn catalog(tag: &str) -> &'static Map<String, Value> {
    static EN: OnceLock<Map<String, Value>> = OnceLock::new();
    static CURRENT: OnceLock<Map<String, Value>> = OnceLock::new();
    let source = |t: &str| {
        CATALOGS
            .iter()
            .find(|(c, _)| *c == t)
            .map_or("{}", |(_, s)| *s)
    };
    if tag == "en" {
        EN.get_or_init(|| parse(source("en")))
    } else {
        CURRENT.get_or_init(|| parse(source(tag)))
    }
}

/// The template for `key` in `tag`: that catalog, then English, then the key itself (never a panic).
fn template(tag: &str, key: &str) -> String {
    let lookup = |t: &str| {
        catalog(t)
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let found = if tag == "en" { None } else { lookup(tag) };
    found
        .or_else(|| lookup("en"))
        .unwrap_or_else(|| key.to_string())
}

/// Replaces each `{name}` with its value, in one pass (a value that contains braces is left as is).
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out += &rest[..open];
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => out += &v.to_string(),
                    None => {
                        out.push('{');
                        out += name;
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out += &rest[open..];
                rest = "";
            }
        }
    }
    out + rest
}

pub fn tr(key: &str) -> String {
    template(lang(), key)
}

pub fn trf(key: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(&template(lang(), key), args)
}

/// Plural: `<key>_one` when `n == 1`, `<key>_other` otherwise, with `{n}` filled in. Languages with
/// more plural forms put a text that reads well for every other count in `_other`.
pub fn trn(key: &str, n: u64, args: &[(&str, &dyn Display)]) -> String {
    let form = if n == 1 { "one" } else { "other" };
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(&template(lang(), &format!("{key}_{form}")), &all)
}

/// Columns a character takes in a terminal: 2 for the wide East Asian ranges (CJK, kana, Hangul,
/// full-width forms), 0 for combining marks, 1 otherwise. A simple table, good enough for aligning
/// our own text.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    match u {
        // Combining marks: Latin accents, Arabic vowels, Devanagari signs written over or under
        // a letter, zero-width spaces and joiners.
        0x0300..=0x036F
        | 0x064B..=0x065F
        | 0x0670
        | 0x0900..=0x0902
        | 0x093A
        | 0x093C
        | 0x0941..=0x0948
        | 0x094D
        | 0x0951..=0x0957
        | 0x0962..=0x0963
        | 0x200B..=0x200F => 0,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// Terminal columns of a text (see [`char_width`]).
pub fn width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// The spaces that bring `text` to `columns`.
pub fn pad(text: &str, columns: usize) -> String {
    " ".repeat(columns.saturating_sub(width(text)))
}

/// A message in the user's language and in English: the text goes to the terminal, the English
/// one into `--json` (which never changes with the language).
#[derive(Debug, Clone)]
pub struct Text {
    pub local: String,
    pub en: String,
}

impl Text {
    pub fn new(key: &str, args: &[(&str, &dyn Display)]) -> Text {
        Text {
            local: fill(&template(lang(), key), args),
            en: fill(&template("en", key), args),
        }
    }

    /// A text that is the same in every language (a path, a key).
    pub fn plain(text: &str) -> Text {
        Text {
            local: text.to_string(),
            en: text.to_string(),
        }
    }

    /// `<self>: <detail>`, where the detail (an OS message, say) is the same in both.
    pub fn with(self, detail: impl Display) -> Text {
        Text {
            local: format!("{}: {detail}", self.local),
            en: format!("{}: {detail}", self.en),
        }
    }
}

/// `t!("key")`, `t!("key", name = value, ...)`: the text of `key` in the user's language.
macro_rules! t {
    ($key:expr) => { $crate::i18n::tr($key) };
    ($key:expr, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::i18n::trf($key, &[$((stringify!($name), &$val as &dyn std::fmt::Display)),+])
    };
}

/// `txt!("key", name = value, ...)`: a [`Text`] (user's language and English) for failures.
macro_rules! txt {
    ($key:expr) => { $crate::i18n::Text::new($key, &[]) };
    ($key:expr, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::i18n::Text::new($key, &[$((stringify!($name), &$val as &dyn std::fmt::Display)),+])
    };
}

pub(crate) use {t, txt};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_names_once() {
        let v = "{x}";
        assert_eq!(
            fill("a {k} b {k} {missing}", &[("k", &v)]),
            "a {x} b {x} {missing}"
        );
        assert_eq!(fill("open { brace", &[]), "open { brace");
    }

    #[test]
    fn resolves_tags() {
        assert_eq!(resolve("pt_BR.UTF-8"), Some("pt-BR"));
        assert_eq!(resolve("pt-pt"), Some("pt-BR"));
        assert_eq!(resolve("EN-us"), Some("en"));
        assert_eq!(resolve("xx-YY"), None);
        assert_eq!(resolve(""), None);
    }
}
