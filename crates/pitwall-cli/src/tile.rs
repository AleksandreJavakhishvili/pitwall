//! Component tiles, for their authors (docs/spec/tile-sdk/authoring.md §9):
//! `pitwall extension new --template`, `dev`, and the component part of
//! `check` and `test`. On a folder, without the app, except `dev`'s reload,
//! which asks the running Pitwall over its socket ([`dev_reload`]).
//!
//! The component is read with `wasmparser` (its imports and exports), not
//! run: this binary doesn't link the tile runtime. `pitwall-extension-test`
//! runs tiles in tests.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pitwall_core::extensions::check::{Finding, Level, Report};
use pitwall_core::extensions::{self, Manifest, MANIFEST};
use serde_json::{json, Value};

use crate::args::TileTemplate;

/// The WIT worlds this Pitwall runs, newest first (pitwall-tile-host's
/// `WORLDS`; a test keeps them equal).
pub const WORLDS: [&str; 1] = ["pitwall:tile@0.1.0"];
/// The largest component Pitwall loads (pitwall-tile-host's
/// `limits::MAX_WASM_BYTES`).
pub const MAX_WASM_BYTES: u64 = 20 << 20;
/// The Rust target tiles are built for.
pub const TARGET: &str = "wasm32-wasip2";
/// How to install it.
pub const ADD_TARGET: &str = "rustup target add wasm32-wasip2";
/// The daemon method that loads a development folder into the running
/// Pitwall, or reloads it there keeping the tile's state: params
/// `{"path": <absolute folder>}`, result `{"message"?}`. Served by the app
/// (the tile view's side); a Pitwall without it answers `unknown_method`.
pub const DEV_RELOAD: &str = "extension.devReload";
/// Pitwall's repository, where the SDK comes from until it is published.
pub const REPO: &str = "https://github.com/pitwalls/pitwall";

/// The interfaces of `pitwall:tile@0.1.0` a component may import.
const INTERFACES: [&str; 18] = ["types", "ui", "log", "tasks", "storage", "settings", "agents", "badges", "asks", "net", "sql", "notify", "show", "toolbar", "service", "agent-tools", "files", "status"];

// ---- new --------------------------------------------------------------------

const CARGO: &str = include_str!("../templates/tile/common/Cargo.toml.tmpl");
const GITIGNORE: &str = include_str!("../templates/tile/common/gitignore.tmpl");
const README: &str = include_str!("../templates/tile/common/README.md.tmpl");
const AGENTS: &str = include_str!("../templates/tile/common/AGENTS.md.tmpl");
/// The authoring skill, shipped with the SDK (authoring.md §9.5).
pub const SKILL: &str = include_str!("../../../sdk/pitwall-extension/skills/pitwall-extension-authoring/SKILL.md");
const SKILL_PATH: &str = "skills/pitwall-extension-authoring/SKILL.md";

struct Files {
    manifest: &'static str,
    lib: &'static str,
    test: &'static str,
    schema: Option<&'static str>,
    about: &'static str,
}

fn files_of(t: TileTemplate) -> Files {
    match t {
        TileTemplate::Blank => Files {
            manifest: include_str!("../templates/tile/blank/extension.json.tmpl"),
            lib: include_str!("../templates/tile/blank/lib.rs.tmpl"),
            test: include_str!("../templates/tile/blank/test.rs.tmpl"),
            schema: None,
            about: "A tile that counts clicks and keeps the count: the smallest start.",
        },
        TileTemplate::List => Files {
            manifest: include_str!("../templates/tile/list/extension.json.tmpl"),
            lib: include_str!("../templates/tile/list/lib.rs.tmpl"),
            test: include_str!("../templates/tile/list/test.rs.tmpl"),
            schema: Some(include_str!("../templates/tile/list/input.schema.json.tmpl")),
            about: "A checklist you add to, tick and filter; an agent can open it with a list to show (input.schema.json).",
        },
        TileTemplate::Form => Files {
            manifest: include_str!("../templates/tile/form/extension.json.tmpl"),
            lib: include_str!("../templates/tile/form/lib.rs.tmpl"),
            test: include_str!("../templates/tile/form/test.rs.tmpl"),
            schema: Some(include_str!("../templates/tile/form/input.schema.json.tmpl")),
            about: "A form for notes with a setting and a toast; an agent can open it with a draft for the user to review (input.schema.json).",
        },
    }
}

/// The template's name, as `--template` takes it.
pub fn template_name(t: TileTemplate) -> &'static str {
    match t {
        TileTemplate::Blank => "blank",
        TileTemplate::List => "list",
        TileTemplate::Form => "form",
    }
}

/// A TOML (and JSON) string of `s`.
fn quoted(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// `path` made absolute with its links resolved, though it (or its
/// parents) may not exist yet.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => resolved(parent).join(name),
        _ => std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf()),
    }
}

/// `to` as a path from folder `from`, when both are inside `repo`
/// (`../../sdk/pitwall-extension`); else `to` itself.
fn from_folder(from: &Path, to: &Path, repo: &Path) -> String {
    match (from.strip_prefix(repo), to.strip_prefix(repo)) {
        (Ok(f), Ok(t)) => {
            let up = "../".repeat(f.components().count());
            format!("{up}{}", t.to_string_lossy().replace('\\', "/"))
        }
        _ => to.to_string_lossy().into_owned(),
    }
}

/// Write a tile project from template `t` into `folder` (absent or empty;
/// its name is the extension's id). `sdk`: a checkout of Pitwall's
/// repository to take the SDK and the harness from (instead of the git
/// URL), by relative paths when `folder` is inside it. `skill`: copy the
/// authoring skill into `.agents/` and `.claude/` (else AGENTS.md points
/// to the SDK's). The files written.
pub fn new_project(folder: &Path, name: Option<&str>, t: TileTemplate, sdk: Option<&Path>, skill: bool) -> Result<Vec<PathBuf>, String> {
    let id = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    extensions::check_id(&id).map_err(|e| format!("the folder's name is the extension's id, and {e}: name the folder like my-tile"))?;
    let name = match name {
        Some(n) => n.trim().to_string(),
        None => extensions::starter::name_of(&id),
    };
    if name.is_empty() || name.chars().count() > extensions::MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("the name must be one line of 1–{} characters", extensions::MAX_NAME));
    }
    if folder.exists() {
        let empty = std::fs::read_dir(folder).map_err(|e| format!("{} can't be read: {e}", folder.display()))?.next().is_none();
        if !empty {
            return Err(format!("{} isn't empty: pick a new folder", folder.display()));
        }
    }
    let at = resolved(folder);
    let skill_in_sdk = format!("sdk/pitwall-extension/{SKILL_PATH}");
    let (sdk_dep, harness_dep, skill_at) = match sdk {
        None => (format!("pitwall-extension = {{ git = \"{REPO}\" }}"), format!("pitwall-extension-test = {{ git = \"{REPO}\" }}"), None),
        Some(repo) => {
            let repo = repo.canonicalize().map_err(|e| format!("--sdk {}: {e}", repo.display()))?;
            let sdk = repo.join("sdk/pitwall-extension");
            let harness = repo.join("crates/pitwall-extension-test");
            if !sdk.join("Cargo.toml").is_file() || !harness.join("Cargo.toml").is_file() {
                return Err(format!("--sdk {}: not a checkout of Pitwall's repository (no sdk/pitwall-extension or crates/pitwall-extension-test)", repo.display()));
            }
            let skill_file = from_folder(&at, &repo.join(&skill_in_sdk), &repo);
            (format!("pitwall-extension = {{ path = {} }}", quoted(&from_folder(&at, &sdk, &repo))), format!("pitwall-extension-test = {{ path = {} }}", quoted(&from_folder(&at, &harness, &repo))), Some(skill_file))
        }
    };
    let (skill_note, skill_row) = if skill {
        (
            "Before changing it, read the authoring skill:\n`.agents/skills/pitwall-extension-authoring/SKILL.md` (the same file is in\n`.claude/skills/`). It covers the manifest, the SDK, testing and the\ncommands.".to_string(),
            "| `AGENTS.md`, `.agents/skills/`, `.claude/skills/` | The authoring skill, for the coding agent working here |\n".to_string(),
        )
    } else {
        let at = match skill_at {
            Some(path) => format!("`{path}`"),
            None => format!("`{skill_in_sdk}` in {REPO}"),
        };
        (
            format!("Before changing it, read the authoring skill, the SDK's own copy:\n{at}. It covers the manifest, the SDK, testing and the commands."),
            "| `AGENTS.md` | Where the authoring skill is, for the coding agent working here |\n".to_string(),
        )
    };
    let f = files_of(t);
    let name_str = quoted(&name);
    let name_str = &name_str[1..name_str.len() - 1];
    let files_rows = if f.schema.is_some() { "| `input.schema.json` | What the tile can be opened with (by you or an agent), as a JSON schema |\n" } else { "" };
    let fill = |s: &str| {
        without_docs_markers(s)
            .replace("{{id}}", &id)
            .replace("{{name_str}}", name_str)
            .replace("{{name}}", &name)
            .replace("{{pitwall}}", extensions::PITWALL)
            .replace("{{sdk_dep}}", &sdk_dep)
            .replace("{{harness_dep}}", &harness_dep)
            .replace("{{about}}", f.about)
            .replace("{{files}}", files_rows)
            .replace("{{skill_note}}", &skill_note)
            .replace("{{skill_row}}", &skill_row)
    };
    let mut out: Vec<(String, String)> = vec![
        ("Cargo.toml".into(), fill(CARGO)),
        (MANIFEST.into(), fill(f.manifest)),
        ("src/lib.rs".into(), fill(f.lib)),
        ("tests/tile.rs".into(), fill(f.test)),
        ("README.md".into(), fill(README)),
        ("AGENTS.md".into(), fill(AGENTS)),
        (".gitignore".into(), GITIGNORE.to_string()),
    ];
    if skill {
        out.push((format!(".agents/{SKILL_PATH}"), SKILL.to_string()));
        out.push((format!(".claude/{SKILL_PATH}"), SKILL.to_string()));
    }
    if let Some(s) = f.schema {
        out.push(("input.schema.json".into(), fill(s)));
    }
    let mut written = Vec::new();
    for (rel, text) in out {
        let path = folder.join(&rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} can't be made: {e}", dir.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{} can't be written: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

/// `s` without the lines that mark the website's examples
/// (`// docs:start <name>`, `// docs:end`): the Extensions docs include
/// regions of these templates, and a new project doesn't need the marks.
fn without_docs_markers(s: &str) -> String {
    s.split_inclusive('\n')
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("// docs:start ") || t.trim_end() == "// docs:end")
        })
        .collect()
}

// ---- the toolchain and the build ---------------------------------------------

/// Whether the Rust toolchain used in `dir` has the `wasm32-wasip2`
/// target (rustup reads `dir`'s rust-toolchain.toml).
pub fn target_installed(dir: &Path) -> bool {
    let at = if dir.is_dir() { dir } else { Path::new(".") };
    Command::new("rustc")
        .args(["--print", "target-libdir", "--target", TARGET])
        .current_dir(at)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .is_some_and(|p| p.is_dir())
}

/// What to say when the target is missing.
pub fn missing_target() -> String {
    format!("the Rust target {TARGET} isn't installed: run `{ADD_TARGET}` (nothing else is needed)")
}

/// A build: the component copied to its place in the folder.
#[derive(Debug, Clone)]
pub struct Built {
    pub wasm: PathBuf,
    pub bytes: u64,
    pub took: Duration,
}

/// The tile project in `dir`: its manifest (with tiles) and the component
/// path its tiles name.
fn project(dir: &Path) -> Result<(Manifest, String), String> {
    let m = extensions::read_in_place(dir).map_err(|e| format!("{MANIFEST} can't be used: {e}"))?;
    let wasm = m.contributes.tiles.first().map(|t| t.wasm.clone()).ok_or_else(|| format!("its {MANIFEST} contributes no tiles (\"contributes.tiles\"): `dev` builds tiles; load an adapter extension with Settings → Extensions → Load from folder"))?;
    if !dir.join("Cargo.toml").is_file() {
        return Err("there is no Cargo.toml: `dev` builds a Rust tile project (`pitwall extension new my-tile --template blank` makes one)".into());
    }
    Ok((m, wasm))
}

/// `cargo build --release --target wasm32-wasip2` in `dir`, the compiler's
/// messages on stderr, and the component copied to where the manifest
/// names it.
pub fn build(dir: &Path) -> Result<Built, String> {
    let (_, wasm) = project(dir)?;
    if !target_installed(dir) {
        return Err(missing_target());
    }
    let at = Instant::now();
    let out = Command::new("cargo")
        .args(["build", "--release", "--target", TARGET, "--message-format=json-render-diagnostics"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cargo couldn't be run: {e}"))?;
    if !out.status.success() {
        return Err("the build failed (the compiler's messages are above)".into());
    }
    let built = artifact(&String::from_utf8_lossy(&out.stdout), dir).ok_or("the build made no .wasm: the crate needs crate-type = [\"cdylib\"]")?;
    let to = dir.join(&wasm);
    // Written beside, then renamed: a reader never sees half a file.
    let tmp = to.with_extension("wasm.part");
    std::fs::copy(&built, &tmp).and_then(|_| std::fs::rename(&tmp, &to)).map_err(|e| format!("{} couldn't be copied to {}: {e}", built.display(), to.display()))?;
    let bytes = std::fs::metadata(&to).map(|m| m.len()).unwrap_or(0);
    Ok(Built { wasm: to, bytes, took: at.elapsed() })
}

/// The `.wasm` the package in `dir` built, from cargo's JSON messages.
fn artifact(messages: &str, dir: &Path) -> Option<PathBuf> {
    let want = dir.join("Cargo.toml").canonicalize().ok()?;
    messages
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter(|m| m["manifest_path"].as_str().and_then(|p| Path::new(p).canonicalize().ok()).as_deref() == Some(want.as_path()))
        .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|f| f.as_str().map(PathBuf::from))
        .rfind(|f| f.extension().is_some_and(|e| e == "wasm"))
}

// ---- check ------------------------------------------------------------------

fn add(r: &mut Report, level: Level, text: impl Into<String>) {
    r.findings.push(Finding { level, text: text.into() });
}

/// What a component imports and exports.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Shape {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
}

/// Read a component's top-level imports and exports (nested modules and
/// components are skipped). An error for a core module or a file that
/// isn't WebAssembly.
pub fn inspect(bytes: &[u8]) -> Result<Shape, String> {
    use wasmparser::{Chunk, Encoding, Parser, Payload};
    let mut parser = Parser::new(0);
    let mut data = bytes;
    let mut shape = Shape::default();
    loop {
        let (consumed, payload) = match parser.parse(data, true).map_err(|e| format!("it isn't valid WebAssembly: {e}"))? {
            Chunk::NeedMoreData(_) => return Err("it ends too early: the file is cut off".into()),
            Chunk::Parsed { consumed, payload } => (consumed, payload),
        };
        data = &data[consumed..];
        match payload {
            Payload::Version { encoding: Encoding::Module, .. } => {
                return Err(format!("it's a core WebAssembly module, not a component: build it for {TARGET} (not wasm32-unknown-unknown or wasm32-wasip1)"));
            }
            Payload::ComponentImportSection(reader) => {
                for i in reader {
                    shape.imports.push(i.map_err(|e| e.to_string())?.name.name.to_string());
                }
            }
            Payload::ComponentExportSection(reader) => {
                for e in reader {
                    shape.exports.push(e.map_err(|e| e.to_string())?.name.name.to_string());
                }
            }
            Payload::ModuleSection { unchecked_range, .. } | Payload::ComponentSection { unchecked_range, .. } => {
                // The outer parser has counted it; step over its bytes.
                data = &data[unchecked_range.len().min(data.len())..];
            }
            Payload::End(_) => return Ok(shape),
            _ => {}
        }
    }
}

/// Whether `name` is an import Pitwall provides: an interface of a world
/// it runs, or WASI 0.2.
fn provided(name: &str) -> bool {
    let pitwall = WORLDS.iter().any(|w| {
        let (pkg, ver) = w.split_once('@').unwrap_or((w, ""));
        name.strip_prefix(pkg).and_then(|r| r.strip_prefix('/')).and_then(|r| r.split_once('@')).is_some_and(|(iface, v)| v == ver && INTERFACES.contains(&iface))
    });
    pitwall || (name.starts_with("wasi:") && name.contains("@0.2."))
}

/// The findings about component file `wasm` (named `shown`).
fn check_wasm(wasm: &Path, shown: &str, r: &mut Report) {
    let meta = match std::fs::symlink_metadata(wasm) {
        Ok(m) => m,
        Err(_) => {
            add(r, Level::Fail, format!("its component {shown} isn't built yet: run `pitwall extension dev` (or `cargo build --release --target {TARGET}` and copy the .wasm there)"));
            return;
        }
    };
    if !meta.is_file() {
        add(r, Level::Fail, format!("its component {shown} must be a plain file (not a link or a folder)"));
        return;
    }
    if meta.len() > MAX_WASM_BYTES {
        add(r, Level::Fail, format!("its component {shown} is {} MiB, over the {} MiB Pitwall loads", meta.len() >> 20, MAX_WASM_BYTES >> 20));
        return;
    }
    let bytes = match std::fs::read(wasm) {
        Ok(b) => b,
        Err(e) => return add(r, Level::Fail, format!("its component {shown} can't be read: {e}")),
    };
    let shape = match inspect(&bytes) {
        Ok(s) => s,
        Err(e) => return add(r, Level::Fail, format!("its component {shown}: {e}")),
    };
    add(r, Level::Ok, format!("{shown} is a component ({} KiB, at most {} MiB)", meta.len().div_ceil(1024), MAX_WASM_BYTES >> 20));
    let unknown: Vec<&str> = shape.imports.iter().map(String::as_str).filter(|i| !provided(i)).collect();
    if unknown.is_empty() {
        let pitwall: Vec<&str> = shape.imports.iter().filter(|i| i.starts_with("pitwall:")).map(|i| i.split('/').nth(1).and_then(|r| r.split('@').next()).unwrap_or(i)).collect();
        add(r, Level::Ok, format!("it imports only what {} provides ({}) and WASI 0.2", WORLDS[0], pitwall.join(", ")));
    } else {
        add(r, Level::Fail, format!("it imports {}, which this Pitwall doesn't provide (only {} and WASI 0.2): build it with pitwall-extension for {}", unknown.join(", "), WORLDS[0], WORLDS[0]));
    }
    let guest = WORLDS.iter().map(|w| w.replacen('@', "/guest@", 1)).collect::<Vec<_>>();
    if shape.exports.iter().any(|e| guest.contains(e)) {
        add(r, Level::Ok, format!("it exports {}", guest[0]));
    } else {
        add(r, Level::Fail, format!("it doesn't export {}: it isn't a tile (end src/lib.rs with `pitwall_extension::export!(YourTile);`)", guest[0]));
    }
}

/// Check `path`: an extension folder (the manifest, as for any extension,
/// then its tiles' component, world, imports, limits and input schemas), its
/// `extension.json`, or a component file alone. Offline.
pub fn check(path: &Path) -> Report {
    if path.is_file() && path.extension().is_some_and(|e| e == "wasm") {
        let mut r = Report { folder: path.to_path_buf(), ..Report::default() };
        check_wasm(path, &path.file_name().unwrap_or_default().to_string_lossy(), &mut r);
        return r;
    }
    let folder = match path.file_name() {
        Some(n) if path.is_file() && n == MANIFEST => path.parent().unwrap_or(Path::new(".")),
        _ => path,
    };
    let mut r = extensions::check::check(folder, None);
    let Ok(m) = extensions::read_in_place(folder) else { return r };
    check_icons(&m, &mut r);
    if m.contributes.tiles.is_empty() {
        return r;
    }
    check_tiles(folder, &m, &mut r);
    r
}

/// The manifest's icons (its own, its tiles', views' and web views') are
/// Pitwall's: an unknown name draws the extension icon instead.
fn check_icons(m: &Manifest, r: &mut Report) {
    let c = &m.contributes;
    let named = std::iter::once(("the extension", m.icon.as_deref()))
        .chain(c.tiles.iter().map(|t| ("a tile", t.icon.as_deref())))
        .chain(c.views.iter().map(|v| ("a view", v.icon.as_deref())))
        .chain(c.web_views.iter().map(|w| ("a web view", w.icon.as_deref())));
    for (what, icon) in named {
        let Some(icon) = icon else { continue };
        if !extensions::ICON_NAMES.contains(&icon) {
            add(r, Level::Warn, format!("{what}'s icon “{icon}” isn't one of Pitwall's, so it shows the extension icon: use one of {}", extensions::ICON_NAMES.join(", ")));
        }
    }
}

fn check_tiles(folder: &Path, m: &Manifest, r: &mut Report) {
    let tiles = &m.contributes.tiles;
    let first = &tiles[0];
    if tiles.iter().any(|t| t.wasm != first.wasm) {
        add(r, Level::Fail, "its tiles name different .wasm files: an extension has one component, and each tile is a view it renders");
    }
    for t in tiles {
        match &t.world {
            Some(w) if WORLDS.contains(&w.as_str()) => {}
            Some(w) => add(r, Level::Fail, format!("tile “{}” is for {w}, which this Pitwall doesn't run (it runs {})", t.id, WORLDS.join(", "))),
            None => add(r, Level::Warn, format!("tile “{}” doesn't say its world: add \"world\": \"{}\" so a Pitwall that doesn't run it says so plainly", t.id, WORLDS[0])),
        }
        if let Some(schema) = &t.input {
            check_schema(folder, &t.id, schema, r);
        }
    }
    let views: Vec<String> = tiles.iter().map(|t| format!("“{}” ({})", t.title, t.id)).collect();
    add(r, Level::Ok, format!("its tiles: {}", views.join(", ")));
    for p in &m.permissions {
        add(r, Level::Ok, format!("it asks for “{}”: {}", p.name, p.reason));
    }
    if let Some(mib) = m.memory_mib {
        add(r, Level::Ok, format!("with “memory” granted its tiles get {mib} MiB (else {} MiB)", extensions::TILE_MEMORY_MIB));
    }
    check_wasm(&folder.join(&first.wasm), &first.wasm, r);
    if let Ok(cargo) = std::fs::read_to_string(folder.join("Cargo.toml")) {
        if !cargo.contains("cdylib") {
            add(r, Level::Warn, "its Cargo.toml doesn't set crate-type = [\"cdylib\"]: the build makes no component");
        }
    }
}

/// A tile's input schema: there, JSON, an object schema.
fn check_schema(folder: &Path, tile: &str, schema: &str, r: &mut Report) {
    let text = match std::fs::read_to_string(folder.join(schema)) {
        Ok(t) => t,
        Err(_) => return add(r, Level::Fail, format!("tile “{tile}”: its input schema {schema} isn't there")),
    };
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return add(r, Level::Fail, format!("tile “{tile}”: its input schema {schema} isn't JSON: {e}")),
    };
    let Value::Object(o) = &v else { return add(r, Level::Fail, format!("tile “{tile}”: its input schema {schema} must be a JSON object (a JSON schema)")) };
    match o.get("type").and_then(Value::as_str) {
        Some("object") => add(r, Level::Ok, format!("tile “{tile}” takes content: {schema} parses as a JSON schema")),
        Some(other) => add(r, Level::Warn, format!("tile “{tile}”: its input schema's type is “{other}”; agents pass content as an object, so make it \"object\"")),
        None => add(r, Level::Warn, format!("tile “{tile}”: its input schema has no \"type\": say \"type\": \"object\" and describe each property for the agents that read it")),
    }
}

// ---- test -------------------------------------------------------------------

/// `pitwall extension test` on a tile project: build it, check it, then
/// `cargo test` (the project's tests, with the harness). The tests' output
/// goes to stderr.
pub fn test(folder: &Path) -> Report {
    let mut r = Report { folder: folder.to_path_buf(), ..Report::default() };
    if let Ok(m) = extensions::read_in_place(folder) {
        (r.id, r.name) = (Some(m.id.clone()), Some(m.name.clone()));
    }
    match build(folder) {
        Ok(b) => add(&mut r, Level::Ok, format!("built {} ({} KiB) in {:.1} s", b.wasm.file_name().unwrap_or_default().to_string_lossy(), b.bytes.div_ceil(1024), b.took.as_secs_f32())),
        Err(e) => {
            add(&mut r, Level::Fail, e);
            return r;
        }
    }
    let c = check(folder);
    let failed: Vec<String> = c.findings.iter().filter(|f| f.level == Level::Fail).map(|f| f.text.clone()).collect();
    if failed.is_empty() {
        add(&mut r, Level::Ok, "`pitwall extension check` passes");
    } else {
        for f in failed {
            add(&mut r, Level::Fail, f);
        }
        return r;
    }
    let out = Command::new("cargo").arg("test").current_dir(folder).stdin(Stdio::null()).stderr(Stdio::inherit()).output();
    match out {
        Ok(o) => {
            let _ = std::io::stderr().write_all(&o.stdout);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let summary: Vec<&str> = stdout.lines().filter(|l| l.starts_with("test result:")).collect();
            let summary = if summary.is_empty() { String::new() } else { format!(" ({})", summary.join("; ").replace("test result: ", "")) };
            if o.status.success() {
                add(&mut r, Level::Ok, format!("cargo test passed{summary}"));
            } else {
                add(&mut r, Level::Fail, format!("cargo test failed{summary}: see its output above"));
            }
        }
        Err(e) => add(&mut r, Level::Fail, format!("cargo couldn't be run: {e}")),
    }
    r
}

// ---- dev --------------------------------------------------------------------

/// What asking the running Pitwall to (re)load a development folder did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reload {
    /// Loaded or reloaded; Pitwall's words, if it sent any.
    Loaded(String),
    /// No Pitwall answers on the socket.
    NotRunning,
    /// The running Pitwall has no [`DEV_RELOAD`] yet.
    Unsupported,
    /// It answered with an error.
    Failed(String),
}

impl Reload {
    fn status(&self) -> &'static str {
        match self {
            Reload::Loaded(_) => "loaded",
            Reload::NotRunning => "notRunning",
            Reload::Unsupported => "unsupported",
            Reload::Failed(_) => "failed",
        }
    }

    fn words(&self) -> String {
        match self {
            Reload::Loaded(m) if m.is_empty() => "Loaded into Pitwall.".into(),
            Reload::Loaded(m) => format!("Loaded into Pitwall: {m}"),
            Reload::NotRunning => "Pitwall isn't running: start it, and the next build loads there.".into(),
            Reload::Unsupported => format!("The running Pitwall can't load development tiles yet (it has no {DEV_RELOAD}): update Pitwall. Built and checked anyway."),
            Reload::Failed(e) => format!("Pitwall didn't load it: {e}"),
        }
    }
}

/// Ask the Pitwall listening on `socket` to load `folder` as a development
/// extension, or reload it there keeping the tile's state.
pub fn dev_reload(socket: &Path, folder: &Path) -> Reload {
    use pitwall_client::{Client, Error};
    let mut c = match Client::connect(socket) {
        Ok(c) => c,
        Err(Error::Connect { .. }) => return Reload::NotRunning,
        Err(e) => return Reload::Failed(e.to_string()),
    };
    match c.call(DEV_RELOAD, json!({ "path": folder })) {
        Ok(v) => Reload::Loaded(v["message"].as_str().unwrap_or_default().to_string()),
        Err(Error::Server(e)) if e.code == pitwall_proto::code::UNKNOWN_METHOD || e.code == pitwall_proto::code::UNSUPPORTED => Reload::Unsupported,
        Err(e) => Reload::Failed(e.to_string()),
    }
}

/// One round of `dev`: build, check, (re)load. Its JSON and text.
fn round(folder: &Path, socket: &Path) -> (Value, String, bool) {
    let built = match build(folder) {
        Ok(b) => b,
        Err(e) => {
            let human = format!("Build failed: {e}\n");
            return (json!({ "built": false, "error": e }), human, false);
        }
    };
    let mut human = format!("Built {} ({} KiB) in {:.1} s.\n", built.wasm.file_name().unwrap_or_default().to_string_lossy(), built.bytes.div_ceil(1024), built.took.as_secs_f32());
    let report = check(folder);
    let passed = report.passed();
    let problems: Vec<&Finding> = report.findings.iter().filter(|f| matches!(f.level, Level::Fail | Level::Warn)).collect();
    for f in &problems {
        human.push_str(&format!("  {}  {}\n", if f.level == Level::Fail { "FAIL" } else { "note" }, f.text));
    }
    if !passed {
        human.push_str("The check failed: not loaded.\n");
        return (json!({ "built": true, "bytes": built.bytes, "passed": false, "findings": problems }), human, false);
    }
    let reload = dev_reload(socket, folder);
    human.push_str(&format!("{}\n", reload.words()));
    let message = match &reload {
        Reload::Loaded(m) | Reload::Failed(m) => Some(m.clone()),
        _ => None,
    };
    let j = json!({ "built": true, "bytes": built.bytes, "passed": true, "findings": problems, "reload": reload.status(), "message": message });
    (j, human, true)
}

/// Whether a changed path is worth a rebuild: not the build's own output
/// (`target/`, the component), git's files or an editor's backups.
fn relevant(folder: &Path, wasm: &Path, p: &Path) -> bool {
    let rel = p.strip_prefix(folder).unwrap_or(p);
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
    let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    !(first == "target" || first == ".git" || p == wasm || name.ends_with(".part") || name.ends_with('~') || name.starts_with(".#") || name.ends_with(".swp"))
}

/// `pitwall extension dev`: a round now, then one after each change to the
/// folder (until stopped), or only one with `once`. Each round prints a
/// JSON line (text with `human`). Ok(false) when `once` and the round
/// failed.
pub fn dev(folder: &Path, socket: &Path, once: bool, human: bool, out: &mut impl Write) -> Result<bool, String> {
    let folder = folder.canonicalize().map_err(|e| format!("{}: {e}", folder.display()))?;
    let (_, wasm) = project(&folder)?;
    if !target_installed(&folder) {
        return Err(missing_target());
    }
    let wasm = folder.join(wasm);
    let print = |out: &mut dyn Write, j: &Value, text: &str| {
        let _ = if human { write!(out, "{text}") } else { writeln!(out, "{j}") };
        let _ = out.flush();
    };
    let (j, text, ok) = round(&folder, socket);
    print(out, &j, &text);
    if once {
        return Ok(ok);
    }
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |e: notify::Result<notify::Event>| {
        if let Ok(e) = e {
            let _ = tx.send(e.paths);
        }
    })
    .map_err(|e| format!("the folder can't be watched: {e}"))?;
    notify::Watcher::watch(&mut watcher, &folder, notify::RecursiveMode::Recursive).map_err(|e| format!("the folder can't be watched: {e}"))?;
    if human {
        let _ = writeln!(out, "Watching {} for changes (Ctrl-C to stop).", folder.display());
    }
    loop {
        let paths = rx.recv().map_err(|_| "the watcher stopped".to_string())?;
        if !paths.iter().any(|p| relevant(&folder, &wasm, p)) {
            continue;
        }
        // Saves come in bursts: wait for a quiet moment.
        while rx.recv_timeout(Duration::from_millis(250)).is_ok() {}
        let (j, text, _) = round(&folder, socket);
        print(out, &j, &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
    }

    /// Every interface of the world counts as provided: the fixtures, which
    /// use all of them between them, pass.
    #[test]
    fn every_interface_of_the_world_is_provided() {
        for f in ["hello", "blocks", "questions", "services"] {
            let wasm = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures").join(f).join("tile.wasm");
            let shape = inspect(&std::fs::read(wasm).unwrap()).unwrap();
            assert!(shape.imports.iter().all(|i| provided(i)), "{f}: {shape:?}");
        }
        for i in INTERFACES {
            assert!(provided(&format!("pitwall:tile/{i}@0.1.0")), "{i}");
        }
    }

    /// The constants this crate copies from the tile host.
    #[test]
    fn worlds_and_limits_match_the_tile_host() {
        assert_eq!(WORLDS, pitwall_tile_host::WORLDS);
        assert_eq!(MAX_WASM_BYTES, pitwall_tile_host::limits::MAX_WASM_BYTES);
        assert_eq!(pitwall_tile_host::WORLDS[0], "pitwall:tile@0.1.0");
    }

    #[test]
    fn the_hello_component_passes_and_bad_files_fail() {
        let shape = inspect(&std::fs::read(hello().join("tile.wasm")).unwrap()).unwrap();
        assert!(shape.exports.contains(&"pitwall:tile/guest@0.1.0".to_string()), "{shape:?}");
        assert!(shape.imports.iter().all(|i| provided(i)), "{shape:?}");
        assert!(shape.imports.iter().any(|i| i == "pitwall:tile/storage@0.1.0"));
        let r = check(&hello());
        assert!(r.passed(), "{}", r.text());
        assert!(r.text().contains("tile.wasm is a component"), "{}", r.text());
        assert!(r.text().contains("“memory”: To test the memory grant"), "{}", r.text());
        assert!(check(&hello().join("extension.json")).passed());
        assert!(check(&hello().join("tile.wasm")).passed());
        assert!(inspect(b"\0asm\x01\0\0\0").unwrap_err().contains("core WebAssembly module"));
        assert!(inspect(b"nope").is_err());
        assert!(!provided("pitwall:tile/git@0.1.0") && !provided("pitwall:tile/ui@0.2.0") && !provided("wasi:sockets/tcp@0.3.0"));
        assert!(provided("wasi:cli/stdout@0.2.6") && provided("pitwall:tile/ui@0.1.0") && provided("pitwall:tile/net@0.1.0") && provided("pitwall:tile/files@0.1.0"));
    }

    #[test]
    fn a_folder_with_a_broken_tile_fails_check() {
        let t = pitwall_core::testing::TempDir::new("cli-tile-check");
        let dir = t.path().join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), r#"{"v": 2, "id": "broken", "name": "Broken", "version": "0.1.0", "contributes": {"tiles": [{"id": "main", "title": "Main", "wasm": "tile.wasm", "input": "in.json"}]}}"#).unwrap();
        std::fs::write(dir.join("in.json"), "[1]").unwrap();
        let r = check(&dir);
        let text = r.text();
        assert!(!r.passed());
        assert!(text.contains("isn't built yet"), "{text}");
        assert!(text.contains("must be a JSON object"), "{text}");
        assert!(text.contains("doesn't say its world"), "{text}");
        assert!(!text.contains("test/pitwall.json"), "no adapter fixture asked of a tile: {text}");
        std::fs::write(dir.join("tile.wasm"), b"\0asm\x01\0\0\0").unwrap();
        assert!(check(&dir).text().contains("core WebAssembly module"));
    }

    /// Each template, made with this checkout's SDK, builds for
    /// wasm32-wasip2 and passes `check`; with PITWALL_TEST_TEMPLATES=1 its
    /// own tests run too (`pitwall extension test`: they build the native
    /// tile runtime, minutes on a cold cache). Skipped, saying so, when the
    /// toolchain has no wasm32-wasip2 (CI's "Tile SDK" job installs it).
    #[test]
    fn templates_build_pass_check_and_their_own_tests() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        if !target_installed(&repo) {
            eprintln!("skipped: {}", missing_target());
            return;
        }
        let full = std::env::var_os("PITWALL_TEST_TEMPLATES").is_some_and(|v| v == "1");
        let toolchain = std::fs::read_to_string(repo.join("rust-toolchain.toml")).unwrap();
        let channel = toolchain.lines().find_map(|l| l.strip_prefix("channel = ")).unwrap().to_string();
        let target_dir = repo.join("target/tile-templates");
        let t = pitwall_core::testing::TempDir::new("cli-tile-templates");
        for tpl in [TileTemplate::Blank, TileTemplate::List, TileTemplate::Form] {
            let dir = t.path().join(format!("tpl-{}", template_name(tpl)));
            new_project(&dir, None, tpl, Some(&repo), true).unwrap();
            // This checkout's compiler, and one build folder for all.
            std::fs::write(dir.join("rust-toolchain.toml"), format!("[toolchain]\nchannel = {channel}\ntargets = [\"{TARGET}\"]\n")).unwrap();
            std::fs::create_dir_all(dir.join(".cargo")).unwrap();
            std::fs::write(dir.join(".cargo/config.toml"), format!("[build]\ntarget-dir = {}\n", quoted(&target_dir.to_string_lossy()))).unwrap();
            if full {
                let r = test(&dir);
                assert!(r.passed(), "{}", r.text());
            } else {
                build(&dir).unwrap_or_else(|e| panic!("{}: {e}", template_name(tpl)));
                let r = check(&dir);
                assert!(r.passed(), "{}", r.text());
            }
        }
    }

    #[test]
    fn changes_in_target_or_to_the_component_dont_rebuild() {
        let f = Path::new("/p/x");
        let w = f.join("tile.wasm");
        assert!(relevant(f, &w, &f.join("src/lib.rs")));
        assert!(relevant(f, &w, &f.join("extension.json")));
        assert!(!relevant(f, &w, &f.join("target/release/x.wasm")));
        assert!(!relevant(f, &w, &w));
        assert!(!relevant(f, &w, &f.join("tile.wasm.part")));
    }

    #[test]
    fn templates_are_written_and_named() {
        let t = pitwall_core::testing::TempDir::new("cli-tile-new");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for tpl in [TileTemplate::Blank, TileTemplate::List, TileTemplate::Form] {
            let dir = t.path().join(format!("my-{}", template_name(tpl)));
            let files = new_project(&dir, Some("My \"quoted\" tile"), tpl, Some(&repo), true).unwrap();
            assert!(files.iter().any(|f| f.ends_with(".claude/skills/pitwall-extension-authoring/SKILL.md")));
            assert!(files.iter().any(|f| f.ends_with(".agents/skills/pitwall-extension-authoring/SKILL.md")));
            let m = extensions::read_in_place(&dir).unwrap();
            assert_eq!((m.id.as_str(), m.name.as_str()), (dir.file_name().unwrap().to_str().unwrap(), "My \"quoted\" tile"));
            assert_eq!(m.contributes.tiles[0].world.as_deref(), Some(WORLDS[0]));
            let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
            assert!(cargo.contains("crate-type = [\"cdylib\"]") && cargo.contains("sdk/pitwall-extension\" }"), "{cargo}");
            let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
            assert!(!lib.contains("{{") && lib.contains("pitwall_extension::export!"), "{lib}");
            // The website's example marks stay in the templates.
            let test = std::fs::read_to_string(dir.join("tests/tile.rs")).unwrap();
            assert!(!lib.contains("docs:") && !test.contains("docs:"), "{lib}{test}");
            // Before a build: only the component is missing.
            let r = check(&dir);
            let fails: Vec<&str> = r.findings.iter().filter(|f| f.level == Level::Fail).map(|f| f.text.as_str()).collect();
            assert_eq!(fails.len(), 1, "{}", r.text());
            assert!(fails[0].contains("isn't built yet"));
        }
        // With no checkout given: the git URL.
        let dir = t.path().join("from-git");
        new_project(&dir, None, TileTemplate::Blank, None, true).unwrap();
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(cargo.contains(&format!("pitwall-extension = {{ git = \"{REPO}\" }}")));
        assert!(new_project(&dir, None, TileTemplate::Blank, None, true).unwrap_err().contains("isn't empty"));
        assert!(new_project(&t.path().join("Bad_Id"), None, TileTemplate::Blank, None, true).is_err());
    }

    /// A tile made inside the checkout it takes the SDK from names it by
    /// relative paths; without the skill copies, AGENTS.md points to the
    /// SDK's skill.
    #[test]
    fn inside_the_checkout_paths_are_relative_and_the_skill_isnt_copied() {
        let t = pitwall_core::testing::TempDir::new("cli-tile-inside");
        let repo = t.path().join("pitwall");
        std::fs::create_dir_all(repo.join("sdk/pitwall-extension")).unwrap();
        std::fs::create_dir_all(repo.join("crates/pitwall-extension-test")).unwrap();
        std::fs::write(repo.join("sdk/pitwall-extension/Cargo.toml"), "").unwrap();
        std::fs::write(repo.join("crates/pitwall-extension-test/Cargo.toml"), "").unwrap();
        let dir = repo.join("tiles/my-notes");
        let files = new_project(&dir, None, TileTemplate::Blank, Some(&repo), false).unwrap();
        assert!(!files.iter().any(|f| f.to_string_lossy().contains("skills")), "{files:?}");
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("pitwall-extension = { path = \"../../sdk/pitwall-extension\" }"), "{cargo}");
        assert!(cargo.contains("pitwall-extension-test = { path = \"../../crates/pitwall-extension-test\" }"), "{cargo}");
        let agents = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
        assert!(agents.contains("`../../sdk/pitwall-extension/skills/pitwall-extension-authoring/SKILL.md`"), "{agents}");
        let readme = std::fs::read_to_string(dir.join("README.md")).unwrap();
        assert!(!readme.contains(".claude/skills") && !readme.contains("{{"), "{readme}");
        // Outside the checkout: absolute paths, as before.
        let out = t.path().join("elsewhere/my-tile");
        new_project(&out, None, TileTemplate::Blank, Some(&repo), true).unwrap();
        let cargo = std::fs::read_to_string(out.join("Cargo.toml")).unwrap();
        assert!(!cargo.contains("../"), "{cargo}");
    }

    /// Icons a manifest names must be Pitwall's.
    #[test]
    fn unknown_icons_are_warned_about() {
        let t = pitwall_core::testing::TempDir::new("cli-tile-icons");
        let dir = t.path().join("icons");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), r#"{"v": 2, "id": "icons", "name": "Icons", "version": "0.1.0", "icon": "rocket", "contributes": {"tiles": [{"id": "main", "title": "Main", "icon": "page-right", "wasm": "tile.wasm", "world": "pitwall:tile@0.1.0"}]}}"#).unwrap();
        let text = check(&dir).text();
        assert!(text.contains("the extension's icon “rocket” isn't one of Pitwall's"), "{text}");
        assert!(!text.contains("“page-right” isn't"), "{text}");
    }
}
