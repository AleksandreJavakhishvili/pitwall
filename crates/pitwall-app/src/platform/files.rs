//! Clipboard, the folder picker, revealing paths and dropped files: what the
//! Tauri app did with the web clipboard, `tauri-plugin-dialog` and
//! `tauri-plugin-opener`, here over gpui's own platform calls (one seam
//! each, so views don't spread OS details).

use std::path::{Path, PathBuf};

use gpui::{App, AppContext, ClipboardItem, ExternalPaths, PathPromptOptions, SharedString, Task};

/// Copy text (an agent's folder path, a file path, a selection).
pub fn copy_text(cx: &mut App, text: impl Into<String>) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
}

/// The clipboard's text, if it holds any (terminal paste).
pub fn paste_text(cx: &App) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// The native folder picker (New agent, New terminal, onboarding's Add
/// folder; `src/lib/pickFolder.ts`). `None` when cancelled or when no picker
/// could open: callers keep their typed-path field as the fallback.
pub fn pick_folder(cx: &mut App, title: impl Into<SharedString>) -> Task<Option<PathBuf>> {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(title.into()),
    });
    cx.background_spawn(async move {
        match picked.await {
            Ok(Ok(Some(mut paths))) => paths.pop(),
            Ok(Ok(None)) | Err(_) => None,
            Ok(Err(e)) => {
                eprintln!("pitwall: folder picker failed: {e}");
                None
            }
        }
    })
}

/// Show a path in Finder / Explorer / the file manager (the rules library).
pub fn reveal(cx: &App, path: &Path) {
    cx.reveal_path(path);
}

/// Open a URL with the system (links, System Settings panes).
pub fn open_url(cx: &App, url: &str) {
    cx.open_url(url);
}

/// macOS privacy panes (`fullDiskAccess`, `filesAndFolders`), opened for the
/// user to change themselves (Tauri: `open_privacy_settings`).
pub fn open_privacy_settings(cx: &App, kind: &str) -> Result<(), String> {
    let url = pitwall_core::permissions::settings_url(kind)?;
    cx.open_url(url);
    Ok(())
}

/// Files dropped from Finder / Explorer onto a terminal, as text to type
/// there: each path quoted for the shell, separated by spaces, with a
/// trailing space (what Terminal.app and iTerm do). The Tauri app ignored
/// file drops; the terminal view can opt in through this one function.
pub fn dropped_paths_text(paths: &ExternalPaths) -> String {
    shell_words(paths.paths())
}

fn shell_words(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for p in paths {
        out.push_str(&quote(&p.to_string_lossy()));
        out.push(' ');
    }
    out
}

/// POSIX shells: single quotes unless the path is plainly safe. Windows
/// shells: double quotes when it has spaces.
fn quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@%~\\".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    if cfg!(windows) {
        format!("\"{s}\"")
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn dropped_paths_are_quoted_for_the_shell() {
        let got = shell_words(&[
            "/work/alpha/main.rs".into(),
            "/work/my notes/it's.md".into(),
        ]);
        assert_eq!(got, r"/work/alpha/main.rs '/work/my notes/it'\''s.md' ");
    }

    #[test]
    #[cfg(windows)]
    fn dropped_paths_are_quoted_for_the_shell() {
        let got = shell_words(&[r"C:\work\a.rs".into(), r"C:\my notes\b.md".into()]);
        assert_eq!(got, r#"C:\work\a.rs "C:\my notes\b.md" "#);
    }

    #[gpui::test]
    fn copied_text_can_be_pasted(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            copy_text(cx, "/work/alpha");
            assert_eq!(paste_text(cx).as_deref(), Some("/work/alpha"));
        });
    }
}
