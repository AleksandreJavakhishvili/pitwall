//! The Race Engineer (docs/spec/engineer.md): an ordinary agent of any
//! kind Pitwall runs (Claude Code, Codex, Gemini CLI, opencode, Aider, a
//! custom command, …) that Pitwall launches with its own know-how. Nothing
//! here touches the agent's global config (`~/.claude`, `~/.codex`, …):
//! everything lives in Pitwall's data folder or is passed per launch.
//!
//! - **Its own folder**: the engineer works in `<data>/engineer/`. Every
//!   launch writes the persona (`race-engineer/ENGINEER.md` from the skills
//!   folder the app ships) followed by the CLI skill (`pitwall/SKILL.md`)
//!   there as `ENGINEER.md` and as the instruction files agents read on
//!   their own: `AGENTS.md` (Codex, opencode, Amp, Cursor, Copilot, …),
//!   `CLAUDE.md` (Claude Code), `GEMINI.md` (Gemini CLI), `QWEN.md` (Qwen
//!   Code). The user's projects are elsewhere, reached by absolute path.
//! - **Per-launch flags** where an agent documents one, on top: Claude Code
//!   `--append-system-prompt`, Codex `-c developer_instructions=…`, Aider
//!   `--read ENGINEER.md` (Aider reads no instruction file by itself).
//! - **Fallback**: an agent that reads none of these (a custom command, a
//!   kind of the user's own) is told "Read ENGINEER.md in this folder first"
//!   in its first queued prompt ([`first_prompt`]).
//! - **CLI**: the shipped `pitwall-cli`, linked as `pitwall` in
//!   `<data>/bin/engineer/`, goes first on the agent's PATH, so it works even
//!   when the user never installed the command-line tool.
//!
//! Every launch (create, restart, resume) does this again, so an updated
//! app refreshes the files.

use std::path::{Path, PathBuf};

use crate::kind::{AgentKind, CUSTOM};
use crate::model::KindView;
use crate::paths::Paths;

/// The engineer's name in the sidebar.
pub const NAME: &str = "Race Engineer";
/// The persona, relative to the skills folder.
pub const PERSONA: &str = "race-engineer/ENGINEER.md";
/// The CLI skill, relative to the skills folder.
pub const SKILL: &str = "pitwall/SKILL.md";
/// What the engineer's folder holds: the persona and skill, and the same
/// text under the names agents read automatically.
pub const FILES: &[&str] = &["ENGINEER.md", "AGENTS.md", "CLAUDE.md", "GEMINI.md", "QWEN.md"];
/// `engineer.agent`'s value that picks for the user.
pub const AUTO: &str = "auto";
/// Kinds that read one of [`FILES`] from their working folder by
/// themselves, or get it through a flag ([`dress`]).
const READS_FILES: &[&str] = &["claude", "codex", "gemini", "opencode", "qwen", "amp", "cursor", "copilot", "aider"];

/// What the app ships for the engineer, found when it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kit {
    /// The skills folder: [`PERSONA`] and [`SKILL`] inside it.
    pub skills: PathBuf,
    /// The shipped `pitwall-cli` (`None`: this build has none).
    pub cli: Option<PathBuf>,
}

/// One launch of the engineer.
#[derive(Debug, Clone)]
pub struct Launch {
    /// The kind with its per-launch flags added.
    pub kind: AgentKind,
    /// Added to its environment (PATH).
    pub env: Vec<(String, String)>,
    /// Where it works: `<data>/engineer`.
    pub cwd: PathBuf,
}

/// `<data>/engineer`: the engineer's working folder.
pub fn workdir(paths: &Paths) -> PathBuf {
    paths.root().join("engineer")
}

/// `<data>/bin/engineer`: the `pitwall` link.
pub fn cli_dir(paths: &Paths) -> PathBuf {
    paths.root().join("bin").join("engineer")
}

impl Kit {
    pub fn persona_file(&self) -> PathBuf {
        self.skills.join(PERSONA)
    }

    pub fn skill_file(&self) -> PathBuf {
        self.skills.join(SKILL)
    }

    /// The instructions: the persona (`{{WORKDIR}}`, `{{SKILL}}` filled in),
    /// then the skill.
    pub fn instructions(&self, workdir: &Path) -> Result<String, String> {
        let read = |f: PathBuf, what: &str| std::fs::read_to_string(&f).map_err(|e| format!("{what} ({}) is missing: {e}", f.display()));
        let persona = read(self.persona_file(), "the Race Engineer's persona")?;
        let skill = read(self.skill_file(), "the Pitwall skill")?;
        let persona = persona
            .replace("{{WORKDIR}}", &workdir.to_string_lossy())
            .replace("{{SKILL}}", &self.skill_file().to_string_lossy());
        Ok(format!("{}\n\n---\n\n<!-- The Pitwall CLI reference ({SKILL}), as shipped with this Pitwall. -->\n\n{}", persona.trim_end(), skill.trim_start()))
    }

    /// Get everything ready for a launch of `base`: the folder and its
    /// files, the CLI link, the per-launch flags and PATH (`path`: the PATH
    /// it would otherwise get). Writes only inside Pitwall's data folder.
    pub fn launch(&self, paths: &Paths, base: &AgentKind, path: Option<String>) -> Result<Launch, String> {
        let cwd = workdir(paths);
        write_files(&cwd, &self.instructions(&cwd)?).map_err(|e| format!("couldn't write the Race Engineer's folder ({}): {e}", cwd.display()))?;
        let kind = dress(base, &cwd);
        let mut env = vec![];
        if let Some(cli) = &self.cli {
            let dir = link_cli(paths, cli).map_err(|e| format!("couldn't put the pitwall CLI on the Race Engineer's PATH: {e}"))?;
            env.push(("PATH".to_string(), prepend_path(&dir, path.as_deref())));
        }
        Ok(Launch { kind, env, cwd })
    }
}

/// [`FILES`] in `dir`, each `text`; unchanged files are left alone.
fn write_files(dir: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for name in FILES {
        let f = dir.join(name);
        if std::fs::read_to_string(&f).ok().as_deref() != Some(text) {
            std::fs::write(&f, text)?;
        }
    }
    Ok(())
}

/// A short pointer for agents with a documented system-prompt flag.
fn pointer(workdir: &Path, file: &str) -> String {
    format!(
        "You are the Race Engineer for Pitwall. Your instructions and the Pitwall CLI reference are in {} (your working folder is {}); follow them.",
        workdir.join(file).display(),
        workdir.display()
    )
}

/// `base` with its documented per-launch flags, on fresh starts and resumes
/// alike: Claude Code `--append-system-prompt`, Codex
/// `-c developer_instructions="…"` before its own arguments (`codex -c …
/// resume <id>`), Aider `--read <ENGINEER.md>`. Other kinds as they are:
/// the instruction files (or the first prompt) carry it.
pub fn dress(base: &AgentKind, workdir: &Path) -> AgentKind {
    let mut kind = base.clone();
    match base.id.as_str() {
        "claude" => {
            let extra = ["--append-system-prompt".to_string(), pointer(workdir, "CLAUDE.md")];
            kind.new_args.extend(extra.iter().cloned());
            kind.resume_args.extend(extra);
        }
        "codex" => {
            let extra = ["-c".to_string(), format!("developer_instructions={}", toml_string(&pointer(workdir, "AGENTS.md")))];
            kind.new_args.splice(0..0, extra.iter().cloned());
            kind.resume_args.splice(0..0, extra);
        }
        "aider" => {
            let extra = ["--read".to_string(), workdir.join("ENGINEER.md").to_string_lossy().into_owned()];
            kind.new_args.extend(extra.iter().cloned());
            kind.resume_args.extend(extra);
        }
        _ => {}
    }
    kind
}

/// A TOML basic string (JSON's escapes are TOML's, for what JSON emits).
fn toml_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Whether `kind` (an id) gets the instructions without being told.
pub fn reads_instructions(kind: &str) -> bool {
    READS_FILES.contains(&kind)
}

/// The engineer's first prompt: `greeting`, after "read ENGINEER.md" for an
/// agent that doesn't read its instruction files by itself.
pub fn first_prompt(kind: &str, greeting: &str) -> String {
    let greeting = greeting.trim();
    if reads_instructions(kind) {
        return greeting.to_string();
    }
    let read = "Read ENGINEER.md in this folder first: it says who you are and how to use the pitwall CLI.";
    if greeting.is_empty() {
        read.to_string()
    } else {
        format!("{read} Then: {greeting}")
    }
}

/// What the engineer runs as, from `engineer.agent` (`setting`) and
/// `engineer.command` (`command`) and the kinds New agent offers: `(kind
/// id, custom command)`. `auto`: Claude Code when installed, else the first
/// installed agent.
pub fn choose(kinds: &[KindView], setting: &str, command: &str) -> Result<(String, Option<String>), String> {
    let agent = |k: &&KindView| k.id != CUSTOM && k.id != crate::engine::terminals::TERMINAL_KIND;
    match setting.trim() {
        "" | AUTO => {
            let installed: Vec<&KindView> = kinds.iter().filter(agent).filter(|k| k.installed).collect();
            installed
                .iter()
                .find(|k| k.id == "claude")
                .or(installed.first())
                .map(|k| (k.id.clone(), None))
                .ok_or_else(|| "no agent is installed for the Race Engineer to run on (Settings → Agents)".into())
        }
        CUSTOM => match command.trim() {
            "" => Err("the Race Engineer is set to a custom command, but engineer.command is empty (Settings → Agents)".into()),
            c => Ok((CUSTOM.to_string(), Some(c.to_string()))),
        },
        id => kinds
            .iter()
            .filter(agent)
            .find(|k| k.id == id)
            .map(|k| (k.id.clone(), None))
            .ok_or_else(|| format!("the Race Engineer is set to \"{id}\", which isn't an agent Pitwall knows (Settings → Agents)")),
    }
}

/// The folder with a `pitwall` that runs `cli`: a symlink (a copy on
/// Windows, as the installed CLI is). Left alone when it is already right.
pub fn link_cli(paths: &Paths, cli: &Path) -> std::io::Result<PathBuf> {
    let dir = cli_dir(paths);
    std::fs::create_dir_all(&dir)?;
    crate::platform::link_program(cli, &dir.join(format!("pitwall{}", std::env::consts::EXE_SUFFIX)))?;
    Ok(dir)
}

/// `dir` first, then `rest` (this process's PATH when `None`).
pub fn prepend_path(dir: &Path, rest: Option<&str>) -> String {
    let rest = rest.map(str::to_string).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    let mut parts = vec![dir.to_path_buf()];
    parts.extend(std::env::split_paths(&rest).filter(|p| p != dir));
    std::env::join_paths(parts).map(|p| p.to_string_lossy().into_owned()).unwrap_or(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::launch::plan;
    use crate::kind::load_kinds;
    use crate::model::KindCaps;
    use crate::testing::TempDir;

    fn builtin(id: &str) -> AgentKind {
        load_kinds(Path::new("/nonexistent/pitwall-agents")).into_iter().find(|k| k.id == id).unwrap()
    }

    fn kit(dir: &Path) -> Kit {
        let skills = dir.join("skills");
        std::fs::create_dir_all(skills.join("pitwall")).unwrap();
        std::fs::create_dir_all(skills.join("race-engineer")).unwrap();
        std::fs::write(skills.join(SKILL), "# Pitwall\nUse `pitwall agent list`.\n").unwrap();
        std::fs::write(skills.join(PERSONA), "You are the race engineer.\nYou work in {{WORKDIR}}.\n").unwrap();
        let cli = dir.join("app").join("pitwall-cli");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, "#!/bin/sh\n").unwrap();
        Kit { skills, cli: Some(cli) }
    }

    fn view(id: &str, installed: bool) -> KindView {
        KindView { id: id.into(), name: id.into(), installed, path: None, worktree: false, caps: KindCaps::default() }
    }

    #[test]
    fn every_kind_gets_the_instruction_files_in_its_own_folder() {
        let dir = TempDir::new("eng-files");
        let k = kit(dir.path());
        let paths = Paths::new(dir.path().join("data"));
        for id in ["claude", "codex", "gemini", "opencode", "aider", "shell"] {
            let l = k.launch(&paths, &builtin(id), Some("/usr/bin".into())).unwrap();
            assert_eq!(l.cwd, paths.root().join("engineer"));
        }
        let cwd = workdir(&paths);
        let want = std::fs::read_to_string(cwd.join("ENGINEER.md")).unwrap();
        assert!(want.starts_with(&format!("You are the race engineer.\nYou work in {}.", cwd.display())), "{want}");
        assert!(want.contains("Use `pitwall agent list`."), "the skill follows the persona");
        for f in FILES {
            assert_eq!(std::fs::read_to_string(cwd.join(f)).unwrap(), want, "{f}");
        }
        // Refreshed from the bundled copies on the next launch.
        std::fs::write(k.persona_file(), "Updated persona.\n").unwrap();
        k.launch(&paths, &builtin("gemini"), None).unwrap();
        assert!(std::fs::read_to_string(cwd.join("GEMINI.md")).unwrap().starts_with("Updated persona."));
    }

    #[test]
    fn launch_args_per_kind() {
        let dir = TempDir::new("eng-args");
        let k = kit(dir.path());
        let paths = Paths::new(dir.path().join("data"));
        let cwd = workdir(&paths);
        let line = |id: &str, resume: Option<&str>| {
            let l = k.launch(&paths, &builtin(id), Some("/usr/bin".into())).unwrap();
            plan(&l.kind, resume, resume.is_some(), "sh '/h'", None).command_line
        };
        // Claude Code: its hooks and session id as always, plus a pointer.
        let c = line("claude", None);
        assert!(c.starts_with("claude --session-id "), "{c}");
        assert!(c.contains(&format!(" --append-system-prompt 'You are the Race Engineer for Pitwall. Your instructions and the Pitwall CLI reference are in {}", cwd.join("CLAUDE.md").display())), "{c}");
        assert!(c.contains(" --settings '"), "{c}");
        let c = line("claude", Some("abc"));
        assert!(c.starts_with("claude --resume abc --append-system-prompt "), "{c}");
        // Codex: a config override for this run only, before `resume`.
        let x = line("codex", None);
        assert!(x.starts_with("codex -c 'developer_instructions=\"You are the Race Engineer for Pitwall."), "{x}");
        assert!(x.contains(&cwd.join("AGENTS.md").display().to_string()), "{x}");
        let x = line("codex", Some("s1"));
        assert!(x.starts_with("codex -c 'developer_instructions=") && x.ends_with("' resume s1"), "{x}");
        // Aider reads no instruction file by itself: --read.
        assert_eq!(line("aider", None), format!("aider --read {}", crate::shell::quote(&cwd.join("ENGINEER.md").to_string_lossy())));
        // The rest read AGENTS.md / GEMINI.md / QWEN.md from their folder.
        assert_eq!(line("gemini", None), "gemini");
        assert_eq!(line("opencode", None), "opencode");
        assert_eq!(line("qwen", Some("q")), "qwen --resume q");
        // A custom command runs as written.
        let custom = k.launch(&paths, &crate::kind::custom_kind("my-agent --fast"), None).unwrap();
        assert_eq!(plan(&custom.kind, None, false, "", None).command_line, "my-agent --fast");
    }

    #[test]
    fn the_codex_value_is_a_toml_string() {
        let k = dress(&builtin("codex"), Path::new("/d/engineer"));
        let v = &k.new_args[1]["developer_instructions=".len()..];
        let parsed: toml::Value = toml::from_str(&format!("x = {v}")).unwrap();
        assert_eq!(parsed["x"].as_str().unwrap(), pointer(Path::new("/d/engineer"), "AGENTS.md"));
    }

    #[test]
    fn the_cli_goes_first_on_path_and_only_pitwalls_folder_is_written() {
        let dir = TempDir::new("eng-path");
        let k = kit(dir.path());
        let data = dir.path().join("data");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let paths = Paths::new(&data);
        let rest = std::env::join_paths(["/usr/bin", "/bin"]).unwrap().to_string_lossy().into_owned();
        let l = k.launch(&paths, &builtin("claude"), Some(rest.clone())).unwrap();
        let bin = cli_dir(&paths);
        let want = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&rest))).unwrap().to_string_lossy().into_owned();
        assert_eq!(l.env, vec![("PATH".to_string(), want)]);
        let link = bin.join(format!("pitwall{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(std::fs::read(&link).unwrap(), std::fs::read(k.cli.clone().unwrap()).unwrap(), "runs the shipped CLI");
        // Again: unchanged, no error.
        k.launch(&paths, &builtin("codex"), Some("/usr/bin".into())).unwrap();
        // Nothing outside Pitwall's data folder (no ~/.claude, ~/.codex, …).
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
        let mut written = walk(&data);
        written.sort();
        let mut want: Vec<PathBuf> = FILES.iter().map(|f| workdir(&paths).join(f)).collect();
        want.push(link);
        want.sort();
        assert_eq!(written, want);
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_bare_agent() {
        let dir = TempDir::new("eng-missing");
        let k = kit(dir.path());
        std::fs::remove_file(k.skill_file()).unwrap();
        let paths = Paths::new(dir.path().join("data"));
        let e = k.launch(&paths, &builtin("claude"), None).unwrap_err();
        assert!(e.contains("pitwall") && e.contains("SKILL.md"), "{e}");
        let no_cli = Kit { cli: None, ..kit(dir.path()) };
        assert!(no_cli.launch(&paths, &builtin("claude"), None).unwrap().env.is_empty());
    }

    #[test]
    fn choosing_what_it_runs_on() {
        let kinds = vec![view("claude", false), view("codex", true), view("gemini", true), view("shell", true), view("custom", true)];
        assert_eq!(choose(&kinds, "auto", "").unwrap().0, "codex", "Claude Code isn't installed: the first installed");
        let mut with_claude = kinds.clone();
        with_claude[0].installed = true;
        assert_eq!(choose(&with_claude, "auto", "").unwrap().0, "claude");
        assert_eq!(choose(&kinds, "gemini", "").unwrap(), ("gemini".into(), None));
        assert_eq!(choose(&kinds, "claude", "").unwrap().0, "claude", "chosen by hand even if not found on PATH");
        assert_eq!(choose(&kinds, "custom", " my-agent ").unwrap(), ("custom".into(), Some("my-agent".into())));
        assert!(choose(&kinds, "custom", "").is_err());
        assert!(choose(&kinds, "shell", "").is_err(), "a terminal isn't an agent");
        assert!(choose(&kinds, "nope", "").is_err());
        assert!(choose(&[view("shell", true)], "auto", "").is_err());
    }

    #[test]
    fn the_first_prompt_points_at_the_file_when_needed() {
        assert_eq!(first_prompt("claude", " Hi. "), "Hi.");
        assert_eq!(first_prompt("gemini", "Hi."), "Hi.");
        let p = first_prompt("custom", "Hi.");
        assert!(p.starts_with("Read ENGINEER.md in this folder first") && p.ends_with("Then: Hi."), "{p}");
        assert!(first_prompt("mine", "").starts_with("Read ENGINEER.md"));
    }

    #[test]
    fn prepending_drops_a_duplicate() {
        let join = |p: &[&str]| std::env::join_paths(p).unwrap().to_string_lossy().into_owned();
        assert_eq!(prepend_path(Path::new("/p/bin"), Some(&join(&["/a", "/p/bin", "/b"]))), join(&["/p/bin", "/a", "/b"]));
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() && !p.is_symlink() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
