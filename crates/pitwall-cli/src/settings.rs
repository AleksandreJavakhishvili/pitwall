//! `pitwall settings`: through the running Pitwall when it answers (applied
//! live; `approval` settings wait for the user's OK there), else straight
//! in `ui.json` under the data folder — `safe` settings only, with a lock
//! and an atomic write, keeping everything else in the file.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Map, Value};

use pitwall_client::{Client, Error};
use pitwall_proto::settings::{self, Sensitivity, Setting, Stored, SETTINGS};
use pitwall_proto::{code, ErrorBody, SettingView};

use crate::args::SettingsCmd;
use crate::{out, render, Output};

fn fail(c: &str, msg: impl Into<String>) -> Error {
    Error::Server(ErrorBody::new(c, msg))
}

fn lookup(key: &str) -> Result<&'static Setting, Error> {
    settings::lookup(key).map_err(|e| fail(code::NOT_FOUND, e))
}

/// The value typed for `key`, checked against the registry before anything
/// is sent or written.
fn value_of(def: &Setting, text: &str) -> Result<Value, Error> {
    let v = def.parse_text(text).map_err(|e| fail(code::BAD_PARAMS, e))?;
    def.validate_change(&v).map_err(|e| fail(code::BAD_PARAMS, e))
}

/// Run against Pitwall at `socket`; when nothing answers there, against
/// `ui.json` in `root`.
pub fn run(cmd: &SettingsCmd, socket: &Path, root: &Path) -> Result<Output, Error> {
    // Bad keys and values fail the same way whether Pitwall runs or not.
    match cmd {
        SettingsCmd::List => {}
        SettingsCmd::Get { key } | SettingsCmd::Reset { key } => {
            lookup(key)?;
        }
        SettingsCmd::Set { key, value } => {
            value_of(lookup(key)?, value)?;
        }
    }
    match Client::connect_as(socket, &format!("pitwall-cli/{}", env!("CARGO_PKG_VERSION")), pitwall_proto::Role::Cli) {
        Ok(c) => online(cmd, c),
        Err(Error::Connect { .. }) => offline(cmd, root),
        Err(e) => Err(e),
    }
}

fn online(cmd: &SettingsCmd, mut c: Client) -> Result<Output, Error> {
    Ok(match cmd {
        SettingsCmd::List => {
            let list = c.settings()?;
            out(&list, render::settings(&list, true))
        }
        SettingsCmd::Get { key } => {
            let v = c.setting(key)?;
            out(&v, render::setting(&v))
        }
        SettingsCmd::Set { key, value } => {
            let v = c.set_setting(key, value_of(lookup(key)?, value)?)?;
            out(&v, render::setting(&v))
        }
        SettingsCmd::Reset { key } => {
            let v = c.reset_setting(key)?;
            out(&v, render::setting(&v))
        }
    })
}

/// `ui.json` under `root`.
pub fn ui_file(root: &Path) -> PathBuf {
    root.join("ui.json")
}

/// The file as an object (`{"v":1}` when there is none yet). A file that
/// isn't Pitwall's settings is refused, never overwritten.
fn read(root: &Path) -> Result<Map<String, Value>, Error> {
    let path = ui_file(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({"v": 1}).as_object().cloned().unwrap_or_default()),
        Err(e) => return Err(fail(code::OTHER, format!("can't read {}: {e}", path.display()))),
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| fail(code::CONFLICT, format!("{} isn't valid JSON ({e}); fix it or remove it first", path.display())))?;
    match v {
        Value::Object(m) if m.get("v").and_then(Value::as_u64) == Some(1) => Ok(m),
        _ => Err(fail(code::CONFLICT, format!("{} isn't a version-1 Pitwall settings file; not changing it", path.display()))),
    }
}

/// A setting's view from the file (settings kept by Pitwall itself read as
/// `null`: only the running app knows them).
fn file_view(def: &'static Setting, ui: &Value) -> SettingView {
    let value = match def.read_file(ui) {
        Some(Ok(v)) => v,
        // Pitwall falls back to the default for a value it can't use.
        Some(Err(_)) => def.default_value(),
        None => Value::Null,
    };
    def.view(value)
}

fn offline(cmd: &SettingsCmd, root: &Path) -> Result<Output, Error> {
    Ok(match cmd {
        SettingsCmd::List => {
            let ui = Value::Object(read(root)?);
            let list: Vec<SettingView> = SETTINGS.iter().map(|d| file_view(d, &ui)).collect();
            out(&list, render::settings(&list, false))
        }
        SettingsCmd::Get { key } => {
            let ui = Value::Object(read(root)?);
            let v = file_view(lookup(key)?, &ui);
            out(&v, render::setting(&v))
        }
        SettingsCmd::Set { key, value } => {
            let def = lookup(key)?;
            let v = write(root, def, value_of(def, value)?)?;
            out(&v, render::setting(&v))
        }
        SettingsCmd::Reset { key } => {
            let def = lookup(key)?;
            let v = write(root, def, def.validate_change(&def.default_value()).map_err(|e| fail(code::BAD_PARAMS, e))?)?;
            out(&v, render::setting(&v))
        }
    })
}

/// Change one safe setting in the file, under the lock.
fn write(root: &Path, def: &'static Setting, value: Value) -> Result<SettingView, Error> {
    if def.sensitivity == Sensitivity::Approval || def.stored == Stored::Pitwall {
        return Err(fail(
            code::NOT_RUNNING,
            format!("Pitwall isn't running: start it to change {} (it asks for your approval there)", def.key),
        ));
    }
    std::fs::create_dir_all(root).map_err(|e| fail(code::OTHER, format!("can't create {}: {e}", root.display())))?;
    let _lock = Lock::take(root)?;
    let mut ui = read(root)?;
    def.write_file(&mut ui, &value);
    let path = ui_file(root);
    let tmp = path.with_extension("json.cli.tmp");
    let text = serde_json::to_string_pretty(&Value::Object(ui)).map_err(|e| fail(code::OTHER, e.to_string()))?;
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| fail(code::OTHER, format!("can't write {}: {e}", path.display())))?;
    Ok(def.view(value))
}

/// `ui.json.lock`: one writer at a time among CLIs; a lock left behind by
/// a crashed one is taken over after [`STALE`].
struct Lock(PathBuf);

const STALE: Duration = Duration::from_secs(30);
const WAIT: Duration = Duration::from_secs(3);

impl Lock {
    fn take(root: &Path) -> Result<Lock, Error> {
        let path = root.join("ui.json.lock");
        let start = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(Lock(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let old = File::open(&path)
                        .and_then(|f| f.metadata())
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > STALE);
                    if old {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > WAIT {
                        return Err(fail(code::CONFLICT, format!("{} is locked by another pitwall command; try again", ui_file(root).display())));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(fail(code::OTHER, format!("can't lock {}: {e}", path.display()))),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
