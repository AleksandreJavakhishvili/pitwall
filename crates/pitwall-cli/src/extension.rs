//! `pitwall extension check|test|new` (docs/extensions/checks.md): for an
//! extension's author, on a folder, offline: no socket, no running app.
//! Tiles (Rust components) are `crate::tile`'s: `new --template`, `dev`,
//! and the component part of `check` and `test`.
//! The work is `pitwall_core::extensions::{check, starter}`; Pitwall's own
//! tests run the same checks on the extensions in its repository.
//!
//! The rest ([`setup`]) is the user's extensions in the running Pitwall,
//! over `extension.*`: reading is free, every change asks the user there.

use pitwall_client::Error;
use pitwall_core::extensions::{check, starter};
use pitwall_proto::{code, ErrorBody};
use serde_json::json;

use crate::args::{ConnectionCmd, ExtensionCmd, ServiceWhat};
use crate::Output;

/// Run `cmd`: its output and whether every check passed.
pub fn run(cmd: &ExtensionCmd) -> Result<(Output, bool), Error> {
    Ok(match cmd {
        ExtensionCmd::Check { folder } => report(crate::tile::check(folder)),
        ExtensionCmd::Test { folder } if is_tile_project(folder) => report(crate::tile::test(folder)),
        ExtensionCmd::Test { folder } => report(check::test(folder, None)),
        ExtensionCmd::New { folder, name, template: Some(t), sdk, no_skill, .. } => new_tile(folder, name.as_deref(), *t, sdk.as_deref(), !*no_skill)?,
        ExtensionCmd::New { folder, name, protocol, view, service, .. } => {
            let shape = match (view, service) {
                (_, true) => starter::Shape::Service,
                (true, _) => starter::Shape::View,
                _ => starter::Shape::Inbox,
            };
            let files = starter::write(folder, name.as_deref(), *protocol, shape).map_err(|e| Error::Server(ErrorBody::new(code::BAD_PARAMS, e)))?;
            let m = pitwall_core::extensions::read_in_place(folder).map_err(|e| Error::Server(ErrorBody::new("failed", e)))?;
            let shown = folder.display();
            let mut human = format!("Made {} ({}) in {shown}:\n", m.name, m.id);
            for f in &files {
                human.push_str(&format!("  {}\n", f.strip_prefix(folder).unwrap_or(f).display()));
            }
            human.push_str(&format!("\nNext:\n  pitwall extension check {shown}\n  pitwall extension test {shown}\n  In Pitwall: Settings → Extensions → Load from folder…, then turn it on.\n"));
            let files: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
            (Output { json: json!({ "folder": folder, "id": m.id, "name": m.name, "files": files }), human, failed: None }, true)
        }
        _ => unreachable!("run against Pitwall (setup)"),
    })
}

/// A Rust tile project: a Cargo.toml and a manifest with tiles.
fn is_tile_project(folder: &std::path::Path) -> bool {
    folder.join("Cargo.toml").is_file() && pitwall_core::extensions::read_in_place(folder).is_ok_and(|m| !m.contributes.tiles.is_empty())
}

/// `pitwall extension new <folder> --template …`.
fn new_tile(folder: &std::path::Path, name: Option<&str>, t: crate::args::TileTemplate, sdk: Option<&std::path::Path>, skill: bool) -> Result<(Output, bool), Error> {
    use crate::tile;
    let files = tile::new_project(folder, name, t, sdk, skill).map_err(|e| Error::Server(ErrorBody::new(code::BAD_PARAMS, e)))?;
    let m = pitwall_core::extensions::read_in_place(folder).map_err(|e| Error::Server(ErrorBody::new("failed", e)))?;
    let shown = folder.display();
    let target = tile::target_installed(folder.parent().unwrap_or(folder));
    let mut human = format!("Made {} ({}), a tile from the {} template, in {shown}:\n", m.name, m.id, tile::template_name(t));
    for f in &files {
        human.push_str(&format!("  {}\n", f.strip_prefix(folder).unwrap_or(f).display()));
    }
    if !target {
        human.push_str(&format!("\nFirst: {}.\n", tile::missing_target()));
    }
    human.push_str(&format!("\nNext:\n  cd {shown}\n  pitwall extension dev     build it, check it, load it into Pitwall; again on every save\n  cargo test                its tests, in Pitwall's tile runtime\n"));
    if sdk.is_none() {
        human.push_str(&format!("\nThe SDK comes from {} until it is on crates.io (Cargo.toml says where).\n", tile::REPO));
    }
    let files: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
    let json = json!({ "folder": folder, "id": m.id, "name": m.name, "template": tile::template_name(t), "files": files, "targetInstalled": target, "addTarget": (!target).then_some(tile::ADD_TARGET) });
    Ok((Output { json, human, failed: None }, true))
}

fn report(r: check::Report) -> (Output, bool) {
    let passed = r.passed();
    let mut json = serde_json::to_value(&r).unwrap_or_default();
    json["passed"] = json!(passed);
    (Output { json, human: r.text(), failed: None }, passed)
}

/// `pitwall extension list|status|log|…` against the running Pitwall.
pub fn setup(cmd: &ExtensionCmd, c: &mut pitwall_client::Client) -> Result<Output, Error> {
    use pitwall_proto::extensions::*;
    use pitwall_proto::{caps, method};
    let id_ref = |id: &str, connection: &Option<String>| ExtRef { id: id.to_string(), connection: connection.clone() };
    let act = |c: &mut pitwall_client::Client, id: &str, op: ExtOp| -> Result<Output, Error> {
        let done: ExtActed = c.call_cap(caps::EXTENSIONS, method::EXTENSION_ACT, ExtAct { id: id.to_string(), op })?;
        let human = acted_text(&done);
        Ok(crate::out(&done, human))
    };
    match cmd {
        ExtensionCmd::List => {
            let l: ExtList = c.call_cap(caps::EXTENSIONS, method::EXTENSION_LIST, ExtRef::default())?;
            Ok(crate::out(&l, list_text(&l)))
        }
        ExtensionCmd::Status { id, connection } => {
            let s: ExtStatus = c.call_cap(caps::EXTENSIONS, method::EXTENSION_STATUS, id_ref(id, connection))?;
            Ok(crate::out(&s, status_text(&s)))
        }
        ExtensionCmd::Log { id, lines } => {
            let l: ExtLog = c.call_cap(caps::EXTENSIONS, method::EXTENSION_LOG, ExtLogRequest { id: id.clone(), lines: Some(*lines) })?;
            Ok(crate::out(&l, log_text(&l)))
        }
        ExtensionCmd::CheckUpdates { id } => {
            let a: ExtActed = c.call_cap(caps::EXTENSIONS, method::EXTENSION_CHECK_UPDATES, id_ref(id, &None))?;
            Ok(crate::out(&a, acted_text(&a)))
        }
        ExtensionCmd::Secret { id, key, connection, agent } => {
            let req = ExtSecret { id: id.clone(), key: key.clone(), connection: connection.clone(), agent_id: agent.clone().or_else(crate::open::env_agent) };
            let a: ExtActed = c.call_cap(caps::EXTENSIONS, method::EXTENSION_SECRET, req)?;
            Ok(crate::out(&a, acted_text(&a)))
        }
        ExtensionCmd::Install { id } if is_folder_arg(id) => install_folder(c, id),
        ExtensionCmd::Install { id } => act(c, id, ExtOp::Install),
        ExtensionCmd::On { id } => act(c, id, ExtOp::On),
        ExtensionCmd::Off { id } => act(c, id, ExtOp::Off),
        ExtensionCmd::Update { id } => act(c, id, ExtOp::Update),
        ExtensionCmd::Remove { id } => act(c, id, ExtOp::Remove),
        ExtensionCmd::Setup { id, action, connection } => act(c, id, ExtOp::Setup { action: action.clone(), connection: connection.clone() }),
        ExtensionCmd::Service { id, what } => act(c, id, ExtOp::Service { what: service_op(*what) }),
        ExtensionCmd::FirstRun { id } => act(c, id, ExtOp::FirstRun),
        ExtensionCmd::Set { id, key, value, connection } => act(c, id, ExtOp::Set { key: key.clone(), value: value.clone(), connection: connection.clone() }),
        ExtensionCmd::Connection(ConnectionCmd::Add { id, name }) => act(c, id, ExtOp::ConnectionAdd { name: name.clone() }),
        ExtensionCmd::Connection(ConnectionCmd::Rename { id, connection, name }) => act(c, id, ExtOp::ConnectionRename { connection: connection.clone(), name: name.clone() }),
        ExtensionCmd::Connection(ConnectionCmd::Remove { id, connection, default }) => act(c, id, ExtOp::ConnectionRemove { connection: connection.clone(), default: default.clone() }),
        ExtensionCmd::Check { .. } | ExtensionCmd::Test { .. } | ExtensionCmd::New { .. } | ExtensionCmd::Dev { .. } => unreachable!("offline (run, dev)"),
    }
}

/// Whether `pitwall extension install`'s argument names a folder (or an
/// address) rather than an id: ids have no `/`, `\\`, `~` or leading `.`.
pub fn is_folder_arg(arg: &str) -> bool {
    arg.contains('/') || arg.contains('\\') || arg.starts_with('.') || arg.starts_with('~')
}

/// `pitwall extension install <folder>` (docs/spec/tile-sdk/packaging.md
/// §6.3): what the folder is read here and shown, then Pitwall checks
/// and copies it and asks the user, listing its permissions and the
/// preview warning.
fn install_folder(c: &mut pitwall_client::Client, arg: &str) -> Result<Output, Error> {
    use pitwall_proto::extensions::*;
    use pitwall_proto::{caps, method};
    if arg.starts_with("http://") || arg.starts_with("https://") {
        return Err(Error::Server(ErrorBody::new(code::UNSUPPORTED, "Installing from an address isn't there yet: download and unpack it, then run `pitwall extension install <folder>`.")));
    }
    let expanded = match arg.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(rest)).unwrap_or_else(|| arg.into()),
        None => std::path::PathBuf::from(arg),
    };
    let folder = expanded.canonicalize().map_err(|_| Error::Server(ErrorBody::new(code::NOT_FOUND, format!("{arg} isn't there"))))?;
    let m = pitwall_core::extensions::preview::check(&folder).map_err(|why| Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("{arg} can't be installed: {why}"))))?;
    // What the user is asked about in Pitwall, shown here too.
    eprintln!("{} {} from {}:", m.name, m.version.clone().unwrap_or_default(), folder.display());
    for l in pitwall_core::extensions::preview::review_lines(&m) {
        eprintln!("  {l}");
    }
    eprintln!("Asking you in Pitwall…");
    let done: ExtActed = c.call_cap(caps::EXTENSIONS, method::EXTENSION_ACT, ExtAct { id: m.id.clone(), op: ExtOp::InstallFolder { path: folder.to_string_lossy().into_owned() } })?;
    let human = acted_text(&done);
    Ok(crate::out(&done, human))
}

/// The service step asked for, as the protocol says it.
pub fn service_op(w: ServiceWhat) -> pitwall_proto::extensions::ServiceOp {
    use pitwall_proto::extensions::ServiceOp;
    match w {
        ServiceWhat::Confirm => ServiceOp::Confirm,
        ServiceWhat::Start => ServiceOp::Start,
        ServiceWhat::Stop => ServiceOp::Stop,
        ServiceWhat::Restart => ServiceOp::Restart,
        ServiceWhat::Update => ServiceOp::Update,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `install` takes an id, or a folder (a path).
    #[test]
    fn install_tells_a_folder_from_an_id() {
        for f in ["./hello", "../x", "/tmp/hello", "~/ext", "dist/hello", ".", "https://example.com/x"] {
            assert!(is_folder_arg(f), "{f}");
        }
        for id in ["inbox", "hello-tile", "my_ext"] {
            assert!(!is_folder_arg(id), "{id}");
        }
    }

    /// The starter, made and checked as an author would.
    #[test]
    fn new_then_check_and_test() {
        let t = pitwall_core::testing::TempDir::new("cli-ext");
        let folder = t.path().join("hello-ext");
        let (o, ok) = run(&ExtensionCmd::New { folder: folder.clone(), name: Some("Hello".into()), protocol: 1, view: false, service: false, template: None, sdk: None, no_skill: false }).unwrap();
        assert!(ok);
        assert_eq!((o.json["id"].as_str(), o.json["name"].as_str()), (Some("hello-ext"), Some("Hello")));
        assert!(o.human.contains("pitwall extension check"));
        let (o, ok) = run(&ExtensionCmd::Check { folder: folder.clone() }).unwrap();
        assert!(ok, "{}", o.human);
        assert_eq!(o.json["passed"], json!(true));
        assert!(o.json["findings"].as_array().is_some_and(|f| !f.is_empty()));
        let (o, ok) = run(&ExtensionCmd::Test { folder: folder.clone() }).unwrap();
        assert!(ok, "{}", o.human);
        // Made again on the same folder: refused, as bad arguments.
        let Err(Error::Server(e)) = run(&ExtensionCmd::New { folder, name: None, protocol: 1, view: false, service: false, template: None, sdk: None, no_skill: false }) else { panic!("made twice") };
        assert_eq!(e.code, code::BAD_PARAMS);
        // A folder that isn't an extension fails, with exit status 1.
        let (o, ok) = run(&ExtensionCmd::Check { folder: t.path().join("nope") }).unwrap();
        assert!(!ok && o.human.contains("FAIL"));
    }
}
