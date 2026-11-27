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
        ExtensionCmd::Pack { folder, out } => (pack(folder.as_deref().unwrap_or(std::path::Path::new(".")), out.as_deref())?, true),
        ExtensionCmd::Sign { file, key } => (sign(file, key.as_deref())?, true),
        _ => unreachable!("run against Pitwall (setup)"),
    })
}

fn failed(e: impl Into<String>) -> Error {
    Error::Server(ErrorBody::new("failed", e.into()))
}

/// `pitwall extension pack [folder] [-o file]`: the folder checked as
/// `check` does, then packaged ([`pitwall_core::extensions::package::pack`])
/// and written (beside it, then renamed: never half a file).
pub fn pack(folder: &std::path::Path, out: Option<&std::path::Path>) -> Result<Output, Error> {
    use pitwall_core::extensions::package;
    let r = crate::tile::check(folder);
    if !r.passed() {
        return Err(Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("It doesn't pass `pitwall extension check`, so it isn't packed:\n{}", r.text()))));
    }
    let p = package::pack(folder).map_err(|why| Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("It can't be packed: {why}"))))?;
    let file = out.map(std::path::Path::to_path_buf).unwrap_or_else(|| folder.join(package::file_name(&p.manifest)));
    let part = file.with_extension(format!("{}.part", package::EXTENSION));
    std::fs::write(&part, &p.bytes).and_then(|()| std::fs::rename(&part, &file)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        failed(format!("Can't write {}: {e}", file.display()))
    })?;
    let version = p.manifest.version.clone().unwrap_or_default();
    let mut human = format!("Packed {} {version} into {} ({}, sha256 {}):\n", p.manifest.name, file.display(), package::size_words(p.bytes.len() as u64), p.sha256);
    for (name, size) in &p.files {
        human.push_str(&format!("  {name}  {}\n", package::size_words(*size)));
    }
    human.push_str(&format!("\nNext:\n  pitwall extension sign {}\n", file.display()));
    let files: Vec<serde_json::Value> = p.files.iter().map(|(n, s)| json!({ "name": n, "bytes": s })).collect();
    let json = json!({ "file": file, "id": p.manifest.id, "name": p.manifest.name, "version": version, "bytes": p.bytes.len(), "sha256": p.sha256, "files": files });
    Ok(Output { json, human, failed: None })
}

/// `pitwall extension sign <file> [--key k]`: a minisign signature of the
/// package's bytes with the author's secret key, written beside it as
/// `<file>.minisig` with the public key in its untrusted comment, then
/// checked as Pitwall checks it.
pub fn sign(file: &std::path::Path, key: Option<&std::path::Path>) -> Result<Output, Error> {
    use pitwall_core::extensions::package;
    let bytes = std::fs::read(file).map_err(|e| Error::Server(ErrorBody::new(code::NOT_FOUND, format!("{} can't be read: {e}", file.display()))))?;
    let (m, _) = package::open_bytes(&bytes).map_err(|why| Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("{} isn't a package Pitwall installs, so it isn't signed: {why}", file.display()))))?;
    let key = match key {
        Some(k) => k.to_path_buf(),
        None => std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".minisign/minisign.key")).ok_or_else(|| failed("No --key, and no home folder to look for ~/.minisign/minisign.key in."))?,
    };
    if !key.is_file() {
        return Err(Error::Server(ErrorBody::new(code::NOT_FOUND, format!("There is no secret key at {}: make one with `minisign -G` (keep it out of any repository), or name yours with --key.", key.display()))));
    }
    let text = std::fs::read_to_string(&key).map_err(|e| failed(format!("The key {} can't be read: {e}", key.display())))?;
    // A key without a password, else one with (asked for, or from
    // PITWALL_SIGN_PASSWORD).
    let sk = minisign::SecretKeyBox::from_string(&text)
        .and_then(|b| b.into_unencrypted_secret_key())
        .or_else(|_| minisign::SecretKeyBox::from_string(&text).and_then(|b| b.into_secret_key(std::env::var("PITWALL_SIGN_PASSWORD").ok())))
        .map_err(|e| failed(format!("The key {} can't be used: {e}", key.display())))?;
    let pk = minisign::PublicKey::from_secret_key(&sk).map_err(|e| failed(format!("The key {} can't be used: {e}", key.display())))?;
    let signer = package::Signer::from_public_key(&pk.to_base64()).map_err(failed)?;
    let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    let trusted = format!("timestamp:{at}\tfile:{name}\tpitwall:{}@{}", m.id, m.version.clone().unwrap_or_default());
    let sig = minisign::sign(Some(&pk), &sk, std::io::Cursor::new(&bytes), Some(&trusted), Some(&package::signature_comment(&signer))).map_err(|e| failed(format!("Signing failed: {e}")))?;
    let to = package::signature_path(file);
    let part = to.with_extension("minisig.part");
    std::fs::write(&part, sig.into_string()).and_then(|()| std::fs::rename(&part, &to)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        failed(format!("Can't write {}: {e}", to.display()))
    })?;
    // Checked as Pitwall will.
    let back = package::open(file, None).map_err(|why| failed(format!("The signature was written but doesn't check: {why}")))?;
    if back.signer.as_ref() != Some(&signer) {
        return Err(failed("The signature was written but names another key."));
    }
    let human = format!(
        "Signed {} {} with key {}: {}\nPublic key (publish it where your users can compare it): {}\nShip both files together; Pitwall checks the signature before it unpacks anything.\n",
        m.name,
        m.version.clone().unwrap_or_default(),
        signer.key_id,
        to.display(),
        signer.public_key
    );
    Ok(Output { json: json!({ "file": file, "signature": to, "keyId": signer.key_id, "publicKey": signer.public_key, "sha256": back.sha256 }), human, failed: None })
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
        ExtensionCmd::Install { id, unsigned, new_key } if is_package_arg(id) => install_package(c, id, *unsigned, *new_key),
        ExtensionCmd::Install { id, .. } if is_folder_arg(id) => install_folder(c, id),
        ExtensionCmd::Install { id, .. } => act(c, id, ExtOp::Install),
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
        ExtensionCmd::Tools { id } => crate::tile_tools::tools(c, id.as_deref()),
        ExtensionCmd::Commands { id } => crate::tile_tools::commands(c, id.as_deref()),
        ExtensionCmd::Call { id, tool, input, timeout } => crate::tile_tools::call_tool(c, id, tool, input, *timeout),
        ExtensionCmd::Run { id, command, json, timeout, props } => crate::tile_tools::run(c, id, command, json.as_deref(), props, *timeout),
        ExtensionCmd::Check { .. } | ExtensionCmd::Test { .. } | ExtensionCmd::New { .. } | ExtensionCmd::Dev { .. } | ExtensionCmd::Pack { .. } | ExtensionCmd::Sign { .. } => unreachable!("offline (run, dev)"),
    }
}

/// Whether `pitwall extension install`'s argument names a package file
/// (`….pwtile`).
pub fn is_package_arg(arg: &str) -> bool {
    arg.to_ascii_lowercase().ends_with(&format!(".{}", pitwall_core::extensions::package::EXTENSION))
}

/// `~/x` as a path under the home folder.
fn expand(arg: &str) -> std::path::PathBuf {
    match arg.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(rest)).unwrap_or_else(|| arg.into()),
        None => std::path::PathBuf::from(arg),
    }
}

/// `pitwall extension install <file.pwtile>` (docs/spec/tile-sdk/
/// packaging.md §6.3): the package read here and its review shown (its
/// signature, what it adds and asks for), then Pitwall checks it again
/// against the installed version's key and asks the user.
fn install_package(c: &mut pitwall_client::Client, arg: &str, unsigned: bool, new_key: bool) -> Result<Output, Error> {
    use pitwall_core::extensions::package;
    use pitwall_proto::extensions::*;
    use pitwall_proto::{caps, method};
    if arg.starts_with("http://") || arg.starts_with("https://") {
        return Err(Error::Server(ErrorBody::new(code::UNSUPPORTED, "Installing from an address isn't there yet: download the .pwtile and its .minisig, then run `pitwall extension install <file>`.")));
    }
    let file = expand(arg).canonicalize().map_err(|_| Error::Server(ErrorBody::new(code::NOT_FOUND, format!("{arg} isn't there"))))?;
    let bytes = std::fs::read(&file).map_err(|e| Error::Server(ErrorBody::new(code::NOT_FOUND, format!("{arg} can't be read: {e}"))))?;
    if bytes.len() as u64 > package::MAX_BYTES {
        return Err(Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("{arg} can't be installed: it is larger than {}.", package::size_words(package::MAX_BYTES)))));
    }
    // The signature first, as Pitwall checks it; one naming a key not in
    // its file is left to Pitwall, which knows the installed version's.
    let sig = std::fs::read_to_string(package::signature_path(&file)).ok();
    let signed = match &sig {
        Some(s) => match package::verify(&bytes, s, None) {
            Ok(signer) => format!("Signed by key {} (public key {}).", signer.key_id, signer.public_key),
            Err(why) if why.contains("public key isn't in the signature file") => "Signed by a key Pitwall checks against the installed version's.".to_string(),
            Err(why) => return Err(Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("{arg} can't be installed: {why}")))),
        },
        None if unsigned => package::UNSIGNED_WARNING.to_string(),
        None => return Err(Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("{arg} isn't signed (there is no {} beside it). Install it only if you trust whoever gave it to you: add --unsigned.", package::signature_path(&file).file_name().unwrap_or_default().to_string_lossy())))),
    };
    let (m, _) = package::open_bytes(&bytes).map_err(|why| Error::Server(ErrorBody::new(code::BAD_PARAMS, format!("{arg} can't be installed: {why}"))))?;
    eprintln!("{} {} from {}:", m.name, m.version.clone().unwrap_or_default(), file.display());
    eprintln!("  {signed}");
    for l in pitwall_core::extensions::preview::review_lines(&m).into_iter().filter(|l| l != pitwall_core::extensions::preview::WARNING) {
        eprintln!("  {l}");
    }
    eprintln!("Asking you in Pitwall…");
    let op = ExtOp::InstallPackage { path: file.to_string_lossy().into_owned(), unsigned, new_key };
    let done: ExtActed = c.call_cap(caps::EXTENSIONS, method::EXTENSION_ACT, ExtAct { id: m.id.clone(), op })?;
    let human = acted_text(&done);
    Ok(crate::out(&done, human))
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

    /// `install` tells a package file from a folder and an id.
    #[test]
    fn install_tells_a_package_from_a_folder() {
        assert!(is_package_arg("./notes-0.1.0.pwtile") && is_package_arg("x.PWTILE") && is_package_arg("~/Downloads/a.pwtile"));
        assert!(!is_package_arg("./notes") && !is_package_arg("notes") && !is_package_arg("a.pwtile.minisig"));
    }

    /// The hello fixture packed (the same file twice), signed with a
    /// throwaway key and checked as Pitwall checks it; a folder that fails
    /// `check`, a file that isn't a package and a missing key are refused.
    #[test]
    fn pack_then_sign() {
        use pitwall_core::extensions::package;
        let t = pitwall_core::testing::TempDir::new("cli-pack");
        let hello = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello");
        let src = t.path().join("hello");
        std::fs::create_dir_all(src.join("assets")).unwrap();
        for f in ["extension.json", "tile.wasm", "input.schema.json", "assets/logo.svg"] {
            std::fs::copy(hello.join(f), src.join(f)).unwrap();
        }
        std::fs::write(src.join("README.md"), "# Hello\n").unwrap();
        let o = pack(&src, None).unwrap();
        let file = src.join("hello-0.1.0.pwtile");
        assert_eq!(o.json["file"].as_str(), Some(file.to_str().unwrap()));
        assert!(o.human.contains("pitwall extension sign"), "{}", o.human);
        let first = std::fs::read(&file).unwrap();
        let out = t.path().join("again.pwtile");
        pack(&src, Some(&out)).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), first, "the same files, the same bytes");
        // Signed with a throwaway key.
        let key = pitwall_core::testing::Signing::new();
        let key_file = t.path().join("test.key");
        std::fs::write(&key_file, key.secret_key_file()).unwrap();
        let o = sign(&file, Some(&key_file)).unwrap();
        assert_eq!(o.json["keyId"].as_str(), Some(key.signer().key_id.as_str()));
        assert!(package::signature_path(&file).is_file());
        assert_eq!(package::open(&file, None).unwrap().signer, Some(key.signer()));
        // Refused: no key there, not a package, a folder that fails check.
        assert!(matches!(sign(&file, Some(&t.path().join("nope.key"))), Err(Error::Server(e)) if e.message.contains("minisign -G")));
        std::fs::write(t.path().join("x.pwtile"), "not a zip").unwrap();
        assert!(matches!(sign(&t.path().join("x.pwtile"), Some(&key_file)), Err(Error::Server(e)) if e.message.contains("isn't a package")));
        std::fs::write(src.join("tile.wasm"), b"\0asm\x01\0\0\0").unwrap();
        assert!(matches!(pack(&src, None), Err(Error::Server(e)) if e.message.contains("check")));
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
