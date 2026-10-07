//! "Open in editor": the user's command line (Settings → Editor, kept in
//! `<data dir>/editor.json`), else the first editor found on the login PATH,
//! else the system opener. Started detached on this computer, never through
//! a shell.

use pitwall_proto::{EditorChoice, EditorSettings};
use serde::{Deserialize, Serialize};

use super::Res;
use crate::paths::Paths;
use crate::platform;

/// Editors looked for when none is set: (id, label, program, arguments).
/// A convenience only; any command line works.
const PRESETS: &[(&str, &str, &str, &str)] = &[
    ("code", "VS Code", "code", "-g {path}:{line}:{column}"),
    ("cursor", "Cursor", "cursor", "-g {path}:{line}:{column}"),
    ("zed", "Zed", "zed", "{path}:{line}:{column}"),
    ("subl", "Sublime Text", "subl", "{path}:{line}:{column}"),
];

#[derive(Serialize, Deserialize, Default)]
struct Stored {
    command: Option<String>,
}

fn load(paths: &Paths) -> Option<String> {
    let text = std::fs::read_to_string(paths.editor_file()).ok()?;
    serde_json::from_str::<Stored>(&text)
        .ok()?
        .command
        .filter(|c| !c.trim().is_empty())
}

pub(super) fn save(paths: &Paths, command: Option<&str>) -> Res<()> {
    let command = command.map(str::trim).filter(|c| !c.is_empty());
    if let Some(c) = command {
        if split_args(c)?.is_empty() {
            return Err("The editor command is empty.".into());
        }
    }
    let file = paths.editor_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(&Stored {
        command: command.map(String::from),
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(&file, json).map_err(|e| format!("{}: {e}", file.display()))
}

/// The system opener as a command line.
fn system_command() -> String {
    let mut parts: Vec<String> = platform::SYSTEM_OPENER
        .iter()
        .map(|s| s.to_string())
        .collect();
    parts.push("{path}".into());
    parts.join(" ")
}

/// Editors found on the login PATH, then the system opener.
fn detected() -> Vec<EditorChoice> {
    let mut found: Vec<EditorChoice> = PRESETS
        .iter()
        .filter(|(_, _, program, _)| platform::which(program).is_some())
        .map(|(id, label, program, args)| EditorChoice {
            id: id.to_string(),
            label: label.to_string(),
            command: format!("{program} {args}"),
        })
        .collect();
    found.push(EditorChoice {
        id: "system".into(),
        label: "System default".into(),
        command: system_command(),
    });
    found
}

pub(super) fn settings(paths: &Paths) -> EditorSettings {
    let command = load(paths);
    let detected = detected();
    let effective = command
        .clone()
        .or_else(|| detected.first().map(|c| c.command.clone()));
    EditorSettings {
        command,
        detected,
        effective,
    }
}

/// A command line split into arguments like a shell would (spaces separate;
/// `'…'` is literal; `"…"` allows `\"` and `\\`), but never run by one.
/// Backslashes elsewhere are kept (Windows paths).
pub fn split_args(line: &str) -> Res<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' => {
                started = true;
                loop {
                    match it.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err("unclosed ' in the editor command".into()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match it.next() {
                        Some('"') => break,
                        Some('\\') if matches!(it.peek(), Some('"') | Some('\\')) => {
                            cur.push(it.next().unwrap_or('\\'))
                        }
                        Some(ch) => cur.push(ch),
                        None => return Err("unclosed \" in the editor command".into()),
                    }
                }
            }
            c if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                started = true;
                cur.push(c);
            }
        }
    }
    if started {
        args.push(cur);
    }
    Ok(args)
}

/// The arguments for opening `path` at `line`/`column` with `template`
/// (`{path}`, `{line}`, `{column}`; no `{path}`: the path goes last).
pub(super) fn command_line(template: &str, path: &str, line: u32, column: u32) -> Res<Vec<String>> {
    let mut args = split_args(template)?;
    if args.is_empty() {
        return Err("The editor command is empty.".into());
    }
    let has_path = args.iter().skip(1).any(|a| a.contains("{path}"));
    for a in args.iter_mut().skip(1) {
        *a = a
            .replace("{path}", path)
            .replace("{line}", &line.to_string())
            .replace("{column}", &column.to_string());
    }
    if !has_path {
        args.push(path.to_string());
    }
    Ok(args)
}

pub(super) fn open(paths: &Paths, path: &str, line: u32, column: u32) -> Res<()> {
    let template = load(paths)
        .or_else(|| detected().into_iter().next().map(|c| c.command))
        .unwrap_or_else(system_command);
    let mut argv = command_line(&template, path, line, column)?;
    // On the login PATH (Windows: `code` is `code.cmd`).
    if !argv[0].contains(['/', '\\']) {
        argv[0] = platform::which(&argv[0])
            .ok_or_else(|| format!("{}: not found (Settings → Editor)", argv[0]))?;
    }
    platform::spawn_detached(&argv).map_err(|e| format!("{}: {e}", argv[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn splits_like_a_shell_without_one() {
        assert_eq!(
            split_args("code -g {path}:{line}").unwrap(),
            ["code", "-g", "{path}:{line}"]
        );
        assert_eq!(
            split_args(r#""C:\Program Files\Ed\ed.exe" --line {line}"#).unwrap(),
            [r"C:\Program Files\Ed\ed.exe", "--line", "{line}"]
        );
        assert_eq!(
            split_args(r#"ed 'a b' "c \"d\"" e\f ''"#).unwrap(),
            ["ed", "a b", "c \"d\"", r"e\f", ""]
        );
        assert_eq!(split_args("  ").unwrap(), Vec::<String>::new());
        assert!(split_args("ed 'open").is_err());
        assert!(split_args("ed \"open").is_err());
    }

    #[test]
    fn placeholders_or_the_path_last() {
        let a = command_line("code -g {path}:{line}:{column}", "/r/a b.rs", 7, 2).unwrap();
        assert_eq!(a, ["code", "-g", "/r/a b.rs:7:2"]);
        let a = command_line("open -t", "/r/$(x);.rs", 1, 1).unwrap();
        assert_eq!(
            a,
            ["open", "-t", "/r/$(x);.rs"],
            "one argument, never a shell"
        );
        assert!(command_line("", "/r/a", 1, 1).is_err());
    }

    #[test]
    fn the_setting_round_trips() {
        let dir = TempDir::new("editor");
        let paths = Paths::new(dir.path().join("pw"));
        let s = settings(&paths);
        assert_eq!(s.command, None);
        assert_eq!(
            s.detected.last().unwrap().id,
            "system",
            "the system opener is always there"
        );
        assert_eq!(s.effective.as_deref(), Some(s.detected[0].command.as_str()));
        save(&paths, Some("  my-ed --wait {path} ")).unwrap();
        let s = settings(&paths);
        assert_eq!(
            (s.command.as_deref(), s.effective.as_deref()),
            (Some("my-ed --wait {path}"), Some("my-ed --wait {path}"))
        );
        assert!(save(&paths, Some("ed 'oops")).is_err());
        save(&paths, None).unwrap();
        assert_eq!(settings(&paths).command, None);
    }
}
