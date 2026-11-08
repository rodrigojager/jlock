//! Registered resources: `resource:<name>[:<arg>]` keys. A catalogue built into the binary (each
//! entry with a localized description, a default mode and whether it takes an argument), plus the
//! user's own entries in `<JLOCK_HOME>/resources.json`. A `resource:` key whose name is in neither is
//! refused, with the closest name as a suggestion: a typo never silently becomes another resource.
//!
//! The user file:
//!
//! ```json
//! { "resources": {
//!     "db":      { "description": "the local test database", "mode": "shared" },
//!     "printer": { "description": { "en": "the office printer", "pt-BR": "a impressora" } },
//!     "vm":      { "description": "a virtual machine, by name", "arg": "name" } } }
//! ```
//!
//! `mode` (`shared` or `exclusive`, default `exclusive`) is the mode taken when the request names
//! none; `arg`, when present, says the resource takes an argument and names it in the help.

use crate::holder::Mode;
use crate::i18n::{t, txt, Text};
use crate::key::{Key, Kind};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The built-in catalogue: name, argument (placeholder), default mode. Descriptions are the catalog
/// entries `resource.<name>`.
pub const BUILTIN: &[(&str, Option<&str>, Mode)] = &[
    ("audio", None, Mode::Exclusive),
    ("browser", None, Mode::Exclusive),
    ("clipboard", None, Mode::Exclusive),
    ("desktop", None, Mode::Exclusive),
    ("gpu", None, Mode::Exclusive),
    ("machine", None, Mode::Exclusive),
    ("network", None, Mode::Exclusive),
    ("port", Some("n"), Mode::Exclusive),
];

#[derive(Clone, Debug)]
pub enum Description {
    /// A catalog entry (built-in resources).
    Catalog(String),
    /// Text from the user file, by language tag (`""` when the file gave one plain string).
    Text(BTreeMap<String, String>),
}

#[derive(Clone, Debug)]
pub struct Resource {
    pub name: String,
    /// The argument's placeholder (`n` for `port:<n>`); `None`: takes no argument.
    pub arg: Option<String>,
    pub mode: Mode,
    pub description: Description,
    pub builtin: bool,
}

impl Resource {
    /// `port:<n>`, `desktop`.
    pub fn label(&self) -> String {
        match &self.arg {
            Some(a) => format!("{}:<{a}>", self.name),
            None => self.name.clone(),
        }
    }

    /// The description in the language of this run (the user's text falls back to English, then
    /// to whatever it has).
    pub fn describe(&self) -> String {
        match &self.description {
            Description::Catalog(key) => t!(key),
            Description::Text(map) => {
                let lang = crate::i18n::lang();
                let primary = |t: &str| t.split('-').next().unwrap_or("").to_ascii_lowercase();
                map.get(lang)
                    .or_else(|| {
                        map.iter()
                            .find(|(k, _)| primary(k) == primary(lang))
                            .map(|(_, v)| v)
                    })
                    .or_else(|| map.get("en"))
                    .or_else(|| map.get(""))
                    .or_else(|| map.values().next())
                    .cloned()
                    .unwrap_or_default()
            }
        }
    }
}

pub struct Catalogue {
    pub resources: Vec<Resource>,
    /// Where the user file is (it may not exist).
    pub file: Option<PathBuf>,
    /// Problems found in the user file, already in the user's language.
    pub warnings: Vec<String>,
}

pub fn file_path() -> Option<PathBuf> {
    crate::home::root().ok().map(|r| r.join("resources.json"))
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c))
}

fn builtin() -> Vec<Resource> {
    BUILTIN
        .iter()
        .map(|(name, arg, mode)| Resource {
            name: name.to_string(),
            arg: arg.map(str::to_string),
            mode: *mode,
            description: Description::Catalog(format!("resource.{name}")),
            builtin: true,
        })
        .collect()
}

/// Reads the user entries of `text`, adding them to `out`; problems go to `warnings`.
pub fn parse_user(text: &str, out: &mut Vec<Resource>, warnings: &mut Vec<String>, path: &str) {
    let root: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(t!("warn.resources_file", path = path, error = e));
            return;
        }
    };
    let Some(entries) = root.get("resources").and_then(Value::as_object) else {
        let e = "no \"resources\" object";
        warnings.push(t!("warn.resources_file", path = path, error = e));
        return;
    };
    for (raw, v) in entries {
        let name = raw.trim().to_ascii_lowercase();
        let bad = |why: &str, warnings: &mut Vec<String>| {
            warnings.push(t!("warn.resources_entry", name = raw, why = why));
        };
        if !valid_name(&name) {
            bad("letters, digits, - _ . only", warnings);
            continue;
        }
        if out.iter().any(|r| r.name == name) {
            bad("already defined", warnings);
            continue;
        }
        let description = match v.get("description") {
            Some(Value::String(s)) => BTreeMap::from([(String::new(), s.clone())]),
            Some(Value::Object(m)) => m
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect(),
            _ => BTreeMap::new(),
        };
        let mode = match v.get("mode").and_then(Value::as_str) {
            None => Mode::Exclusive,
            Some(m) => match Mode::parse(m) {
                Some(m) => m,
                None => {
                    bad("mode is shared or exclusive", warnings);
                    continue;
                }
            },
        };
        let arg = match v.get("arg") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => None,
            Some(Value::Bool(true)) => Some("arg".to_string()),
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(_) => {
                bad("arg is the name of the argument", warnings);
                continue;
            }
        };
        out.push(Resource {
            name,
            arg,
            mode,
            description: Description::Text(description),
            builtin: false,
        });
    }
}

/// The built-in catalogue plus the user's entries.
pub fn load() -> Catalogue {
    let mut resources = builtin();
    let mut warnings = Vec::new();
    let file = file_path();
    if let Some(path) = &file {
        if let Ok(text) = std::fs::read_to_string(path) {
            parse_user(
                &text,
                &mut resources,
                &mut warnings,
                &path.display().to_string(),
            );
        }
    }
    Catalogue {
        resources,
        file,
        warnings,
    }
}

impl Catalogue {
    pub fn find(&self, name: &str) -> Option<&Resource> {
        self.resources.iter().find(|r| r.name == name)
    }

    /// Prints the problems of the user file on stderr.
    pub fn warn(&self) {
        for w in &self.warnings {
            eprintln!("jlock: {w}");
        }
    }

    /// Checks a `resource:` key against the catalogue: the name exists, the argument is there when
    /// the resource takes one and absent otherwise, a port is a number from 1 to 65535. Returns the
    /// key in its canonical form (a port without leading zeros) and the resource's default mode.
    pub fn check(&self, key: &Key) -> Result<(Key, Mode), Text> {
        debug_assert_eq!(key.kind, Kind::Resource);
        let rest = key.text.strip_prefix("resource:").unwrap_or(&key.text);
        let (name, arg) = match rest.split_once(':') {
            Some((n, a)) => (n, Some(a)),
            None => (rest, None),
        };
        let Some(r) = self.find(name) else {
            let names = self.resources.iter().map(|r| r.name.as_str());
            return Err(match crate::help::closest(name, names) {
                Some(near) => txt!(
                    "error.unknown_resource_near",
                    name = name,
                    near = format!("resource:{near}")
                ),
                None => txt!("error.unknown_resource", name = name),
            });
        };
        match (&r.arg, arg) {
            (Some(placeholder), None) => Err(txt!(
                "error.resource_needs_arg",
                name = name,
                arg = format!("resource:{name}:<{placeholder}>")
            )),
            (None, Some(_)) => Err(txt!("error.resource_no_arg", name = name)),
            (Some(_), Some(a)) if r.builtin && name == "port" => match a.parse::<u16>() {
                Ok(n) if n > 0 => Ok((Key::from_text(&format!("resource:port:{n}")), r.mode)),
                _ => Err(txt!("error.resource_bad_port", value = a)),
            },
            _ => Ok((key.clone(), r.mode)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(user: &str) -> Catalogue {
        let mut resources = builtin();
        let mut warnings = Vec::new();
        if !user.is_empty() {
            parse_user(user, &mut resources, &mut warnings, "resources.json");
        }
        Catalogue {
            resources,
            file: None,
            warnings,
        }
    }

    fn check(c: &Catalogue, key: &str) -> Result<(String, Mode), String> {
        c.check(&Key::from_text(key))
            .map(|(k, m)| (k.text, m))
            .map_err(|t| t.en)
    }

    #[test]
    fn builtin_names_and_arguments() {
        let c = cat("");
        assert_eq!(
            check(&c, "resource:desktop"),
            Ok(("resource:desktop".into(), Mode::Exclusive))
        );
        assert_eq!(
            check(&c, "resource:port:08080").unwrap().0,
            "resource:port:8080"
        );
        assert!(check(&c, "resource:port").unwrap_err().contains("port:<n>"));
        assert!(check(&c, "resource:port:http").is_err());
        assert!(check(&c, "resource:port:0").is_err());
        assert!(check(&c, "resource:desktop:1").is_err());
    }

    #[test]
    fn typo_is_refused_with_a_suggestion() {
        let c = cat("");
        let e = check(&c, "resource:desktp").unwrap_err();
        assert!(e.contains("resource:desktop"), "{e}");
        let e = check(&c, "resource:zzzzzzzz").unwrap_err();
        assert!(!e.contains("did you mean"), "{e}");
    }

    #[test]
    fn user_entries() {
        let c = cat(r#"{ "resources": {
                "DB": { "description": "test db", "mode": "shared" },
                "vm": { "description": { "en": "a vm", "pt-BR": "uma vm" }, "arg": "name" },
                "desktop": { "description": "mine" },
                "bad name": {},
                "x": { "mode": "sometimes" } } }"#);
        assert_eq!(c.warnings.len(), 3, "{:?}", c.warnings);
        assert_eq!(
            check(&c, "resource:db"),
            Ok(("resource:db".into(), Mode::Shared))
        );
        assert_eq!(
            check(&c, "resource:vm:web-1").unwrap().0,
            "resource:vm:web-1"
        );
        assert!(check(&c, "resource:vm").is_err());
        assert_eq!(c.find("vm").unwrap().label(), "vm:<name>");
        assert!(c.find("desktop").unwrap().builtin);
    }

    #[test]
    fn broken_file_is_a_warning() {
        let c = cat("{ not json");
        assert_eq!(c.warnings.len(), 1);
        assert_eq!(c.resources.len(), BUILTIN.len());
    }
}
