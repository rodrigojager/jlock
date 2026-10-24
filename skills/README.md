# jlock skills for AI agents

A skill teaches an AI coding agent when and how to use jlock: take the files of a task before editing
them (`jlock hold ... --detach`), give them back at the end (`jlock release`), check a lock after a long
pause (`jlock heartbeat`), run builds and servers one at a time (`jlock run`), and what to do when a file
is taken. The three skills say the same thing; each uses its tool's name in `--owner` and lives where
that tool looks for skills.

| Agent | Skill | Installed for the user at | Or for one project at |
|---|---|---|---|
| Claude Code | [`claude-code/jlock`](claude-code/jlock/SKILL.md) | `~/.claude/skills/jlock/` | `.claude/skills/jlock/` |
| OpenCode | [`opencode/jlock`](opencode/jlock/SKILL.md) | `~/.config/opencode/skills/jlock/` | `.opencode/skills/jlock/` |
| Codex | [`codex/jlock`](codex/jlock/SKILL.md) | `~/.agents/skills/jlock/` | `.agents/skills/jlock/` |

jlock itself has to be installed first (see the [README](../README.md#install)).

## Install with one command

Linux and macOS:

```
# Claude Code
mkdir -p ~/.claude/skills/jlock && curl -fsSL https://raw.githubusercontent.com/rodrigojager/jlock/main/skills/claude-code/jlock/SKILL.md -o ~/.claude/skills/jlock/SKILL.md
# OpenCode
mkdir -p ~/.config/opencode/skills/jlock && curl -fsSL https://raw.githubusercontent.com/rodrigojager/jlock/main/skills/opencode/jlock/SKILL.md -o ~/.config/opencode/skills/jlock/SKILL.md
# Codex
mkdir -p ~/.agents/skills/jlock && curl -fsSL https://raw.githubusercontent.com/rodrigojager/jlock/main/skills/codex/jlock/SKILL.md -o ~/.agents/skills/jlock/SKILL.md
```

Windows (PowerShell):

```
# Claude Code
New-Item -ItemType Directory -Force "$HOME\.claude\skills\jlock" | Out-Null; irm https://raw.githubusercontent.com/rodrigojager/jlock/main/skills/claude-code/jlock/SKILL.md -OutFile "$HOME\.claude\skills\jlock\SKILL.md"
# OpenCode
New-Item -ItemType Directory -Force "$HOME\.config\opencode\skills\jlock" | Out-Null; irm https://raw.githubusercontent.com/rodrigojager/jlock/main/skills/opencode/jlock/SKILL.md -OutFile "$HOME\.config\opencode\skills\jlock\SKILL.md"
# Codex
New-Item -ItemType Directory -Force "$HOME\.agents\skills\jlock" | Out-Null; irm https://raw.githubusercontent.com/rodrigojager/jlock/main/skills/codex/jlock/SKILL.md -OutFile "$HOME\.agents\skills\jlock\SKILL.md"
```

For a single project, put the same folder under the project path from the table instead (and commit it,
so that every agent working on the repository gets it). Start a new session of the agent afterwards:
skills are read when a session starts.

From a clone of this repository, copying the folder works too, for example
`cp -r skills/claude-code/jlock ~/.claude/skills/`.

## Updating

Run the same command again: it replaces `SKILL.md` with the current one.
