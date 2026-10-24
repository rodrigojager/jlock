//! The one table of commands and options. The help is rendered from it (`help.rs`) and the
//! dispatcher refuses any option not in it, so no option can exist without a line of help. Pure
//! data: the tests include this file directly to check the parser and the help against it.

/// One `--option`. Its help text is the catalog entry `help`; `meta` names the catalog entry
/// `help.meta.<meta>` of the value it takes (`None`: a switch).
pub struct Opt {
    pub name: &'static str,
    pub meta: Option<&'static str>,
    pub help: &'static str,
    /// A value that the option accepts, as typed (used in the help and by the tests).
    pub example: &'static str,
}

pub struct Command {
    pub name: &'static str,
    /// Catalog key of the heading the general help lists it under.
    pub group: &'static str,
    pub opts: &'static [Opt],
    /// Exit codes it can return, as catalog suffixes: `help.exit.<cmd>.<code>` if the catalog has
    /// it, else `help.exit.<code>`.
    pub codes: &'static [&'static str],
    pub see: &'static [&'static str],
}

const fn opt(
    name: &'static str,
    meta: Option<&'static str>,
    help: &'static str,
    example: &'static str,
) -> Opt {
    Opt {
        name,
        meta,
        help,
        example,
    }
}

pub const JSON: Opt = opt("json", None, "help.opt.json", "");

/// The options of `acquire`, also taken by `hold` and `run`.
pub const ACQUIRE_OPTS: &[Opt] = &[
    opt("owner", Some("id"), "help.opt.acquire.owner", "agent-7"),
    opt("group", Some("group"), "help.opt.acquire.group", "task-42"),
    opt(
        "intent",
        Some("text"),
        "help.opt.acquire.intent",
        "\"refactor parser\"",
    ),
    opt("meta", Some("kv"), "help.opt.acquire.meta", "task=T04"),
    opt("ttl", Some("time"), "help.opt.acquire.ttl", "2h"),
    opt("wait", None, "help.opt.acquire.wait", ""),
    opt("no-wait", None, "help.opt.acquire.no_wait", ""),
    opt("timeout", Some("time"), "help.opt.acquire.timeout", "30s"),
    JSON,
];

pub const TAKE: &str = "help.group.take";
pub const SEE: &str = "help.group.see";
pub const EMERGENCY: &str = "help.group.emergency";
pub const MAINTENANCE: &str = "help.group.maintenance";
pub const HELP: &str = "help.group.help";

/// The headings of the general help, in order.
pub const GROUPS: &[&str] = &[TAKE, SEE, EMERGENCY, MAINTENANCE, HELP];

pub const COMMANDS: &[Command] = &[
    Command {
        name: "acquire",
        group: TAKE,
        opts: ACQUIRE_OPTS,
        codes: &["0", "2", "10", "11", "12", "20", "130"],
        see: &["run", "hold", "release", "status"],
    },
    Command {
        name: "run",
        group: TAKE,
        opts: ACQUIRE_OPTS,
        codes: &["0", "2", "10", "11", "12", "14", "20", "130"],
        see: &["acquire", "hold"],
    },
    Command {
        name: "hold",
        group: TAKE,
        opts: ACQUIRE_OPTS,
        codes: &["0", "2", "10", "11", "12", "14", "20", "130"],
        see: &["acquire", "run", "release"],
    },
    Command {
        name: "release",
        group: TAKE,
        opts: &[
            opt("owner", Some("id"), "help.opt.release.owner", "agent-7"),
            opt("group", Some("group"), "help.opt.release.group", "task-42"),
            opt("any-owner", None, "help.opt.release.any_owner", ""),
            JSON,
        ],
        codes: &["0", "2", "13", "20"],
        see: &["acquire", "unlock"],
    },
    Command {
        name: "heartbeat",
        group: TAKE,
        opts: &[
            opt("owner", Some("id"), "help.opt.heartbeat.owner", "agent-7"),
            JSON,
        ],
        codes: &["0", "2", "13", "20"],
        see: &["acquire", "hold"],
    },
    Command {
        name: "status",
        group: SEE,
        opts: &[JSON],
        codes: &["0", "2", "20"],
        see: &["list", "graph"],
    },
    Command {
        name: "list",
        group: SEE,
        opts: &[
            opt("owner", Some("id"), "help.opt.list.owner", "agent-7"),
            opt("group", Some("group"), "help.opt.list.group", "task-42"),
            opt("under", Some("path"), "help.opt.list.under", "D:\\proj"),
            opt("stale", None, "help.opt.list.stale", ""),
            opt("watch", None, "help.opt.list.watch", ""),
            JSON,
        ],
        codes: &["0", "2", "20"],
        see: &["status", "graph", "unlock"],
    },
    Command {
        name: "graph",
        group: SEE,
        opts: &[JSON],
        codes: &["0", "2", "20"],
        see: &["list", "status"],
    },
    Command {
        name: "unlock",
        group: EMERGENCY,
        opts: &[
            opt("force", None, "help.opt.unlock.force", ""),
            opt("owner", Some("id"), "help.opt.unlock.owner", "agent-3"),
            opt("group", Some("group"), "help.opt.unlock.group", "task-42"),
            opt(
                "reason",
                Some("text"),
                "help.opt.unlock.reason",
                "\"agent hung\"",
            ),
            opt("by", Some("id"), "help.opt.unlock.by", "ci"),
            JSON,
        ],
        codes: &["0", "2", "20"],
        see: &["release", "list"],
    },
    Command {
        name: "gc",
        group: MAINTENANCE,
        opts: &[JSON],
        codes: &["0", "2", "20"],
        see: &["list"],
    },
    Command {
        name: "doctor",
        group: MAINTENANCE,
        opts: &[JSON],
        codes: &["0", "2", "20"],
        see: &["gc", "list"],
    },
    Command {
        name: "help",
        group: HELP,
        opts: &[],
        codes: &["0", "2"],
        see: &["about"],
    },
    Command {
        name: "about",
        group: HELP,
        opts: &[JSON],
        codes: &["0", "2"],
        see: &["help"],
    },
];

pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

impl Command {
    pub fn opt(&self, name: &str) -> Option<&'static Opt> {
        self.opts.iter().find(|o| o.name == name)
    }
}
