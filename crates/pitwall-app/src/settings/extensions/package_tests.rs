//! Tile packages in the app (docs/spec/tile-sdk/packaging.md §6), headless
//! (GPUI's test platform), with the tile host's hello fixture packed and
//! signed by throwaway keys made in the test: Install from file… refuses
//! a file that isn't a package and one changed after it was signed, shows
//! the key and installs; an update by the same key shows its new
//! permissions, which stay ungranted until allowed; one by another key
//! installs only once "Trust the new key" is ticked; Go back returns the
//! version before; Remove asks about its data and deletes it when ticked.
//! And `pitwall extension install <file>` as the CLI sends it, through the
//! approval.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext};
use pitwall_core::extensions::install::{self, Origin};
use pitwall_core::extensions::package;
use pitwall_core::extensions::permissions;
use pitwall_core::testing::{Signing, TempDir};
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use crate::extensions::catalog;

const WAIT: Duration = Duration::from_secs(60);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
}

/// The hello fixture as an author's folder at `version`, asking for
/// `perms` more (manifest text after the memory permission), packed into
/// `dir`, and signed by `key` (none: unsigned). The package's path.
pub(crate) fn hello_package(dir: &Path, version: &str, perms: &str, key: Option<&Signing>) -> PathBuf {
    let src = dir.join(format!("hello-src-{version}"));
    std::fs::create_dir_all(src.join("assets")).unwrap();
    for f in ["tile.wasm", "input.schema.json", "assets/logo.svg"] {
        std::fs::copy(fixture().join(f), src.join(f)).unwrap();
    }
    let m = std::fs::read_to_string(fixture().join("extension.json")).unwrap();
    let m = m.replace(r#""version": "0.1.0""#, &format!(r#""version": "{version}""#));
    let m = m.replace(r#""mib": 128 }"#, &format!(r#""mib": 128 }}{perms}"#));
    std::fs::write(src.join("extension.json"), m).unwrap();
    std::fs::write(src.join("README.md"), "# Hello\n\nThe tile host's test fixture.\n").unwrap();
    let p = package::pack(&src).unwrap();
    let file = dir.join(package::file_name(&p.manifest));
    std::fs::write(&file, &p.bytes).unwrap();
    if let Some(k) = key {
        std::fs::write(package::signature_path(&file), k.sign(&p.bytes)).unwrap();
    }
    file
}

fn init(root: &Path, cx: &mut TestAppContext) {
    let root = root.to_path_buf();
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(root.clone()), cx);
        catalog::set_bundle(None, cx);
        cx.set_global(crate::tiles::DataDir(root.clone()));
        crate::settings::data::set_page(cx, Page::Extensions);
    });
}

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        frame(cx);
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Look at package `file` and wait until it is read: what the card says.
fn look(view: &gpui::Entity<SettingsView>, file: &Path, cx: &mut VisualTestContext) -> (Option<String>, Option<String>) {
    let f = file.to_path_buf();
    // The installed card folded, so the card under Available is on screen.
    view.update(cx, |v, cx| {
        v.exts.open.insert("hello".into(), false);
        v.look_at_package(f, cx)
    });
    until(cx, "the package read", |cx| view.read_with(cx, |v, _| v.add_file_state().is_some_and(|(busy, _, _)| !busy)));
    let (_, note, id) = view.read_with(cx, |v, _| v.add_file_state()).unwrap();
    (note, id)
}

fn signer_of(exts: &Path) -> Option<package::Signer> {
    match install::origin(&exts.join("hello")) {
        Some(Origin::Package { signer, .. }) => signer,
        _ => None,
    }
}

#[gpui::test]
fn a_package_installs_updates_goes_back_and_is_removed(cx: &mut TestAppContext) {
    let t = TempDir::new("pkg-app");
    let root = t.path().join("data");
    let exts = root.join("extensions");
    std::fs::create_dir_all(&exts).unwrap();
    init(&root, cx);
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(2400.)));
    frame(cx);
    let author = Signing::new();
    let v1 = hello_package(t.path(), "0.1.0", "", Some(&author));

    // Not a package; one changed after it was signed: refused, saying why.
    let (note, id) = look(&view, &fixture().join("extension.json"), cx);
    assert!(id.is_none() && note.as_deref().is_some_and(|n| n.contains("isn't a .pwtile package")), "{note:?}");
    let tampered = t.path().join("tampered.pwtile");
    let mut bytes = std::fs::read(&v1).unwrap();
    let n = bytes.len();
    bytes[n / 3] ^= 0x40;
    std::fs::write(&tampered, &bytes).unwrap();
    std::fs::copy(package::signature_path(&v1), package::signature_path(&tampered)).unwrap();
    let (note, id) = look(&view, &tampered, cx);
    assert!(id.is_none() && note.as_deref().is_some_and(|n| n.contains("doesn't match the file")), "{note:?}");
    assert!(cx.debug_bounds("extension-add-file-note").is_some());

    // Unsigned: shown as such, "Install unsigned".
    let bare = t.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    let unsigned = hello_package(&bare, "0.1.0", "", None);
    let (note, id) = look(&view, &unsigned, cx);
    assert_eq!((note, id.as_deref()), (None, Some("hello")));
    let key = view.update(cx, |v, cx| catalog::package_key(v.exts.add_file.as_ref().unwrap().prepared.as_ref().unwrap(), cx));
    assert_eq!(key, Some(package::Key::Unsigned));
    assert!(cx.debug_bounds("extension-add-file-key-hello").is_some());

    // Signed: the key shown; installed, off, with the key recorded.
    let (note, id) = look(&view, &v1, cx);
    assert_eq!((note, id.as_deref()), (None, Some("hello")));
    let staged: Vec<_> = std::fs::read_dir(&exts).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(staged.len(), 1, "the unsigned one's staged copy went when another was picked: {staged:?}");
    assert!(cx.debug_bounds("extension-add-file-key-hello").is_some());
    view.update(cx, |v, cx| v.install_from_package(cx));
    until(cx, "the install", |cx| view.read_with(cx, |v, _| v.add_file_state().is_none()));
    assert!(exts.join("hello/tile.wasm").is_file() && exts.join("hello/README.md").is_file());
    assert_eq!(signer_of(&exts), Some(author.signer()));
    assert!(cx.update(|_, cx| !crate::ui_state::get(cx).extension_on("hello")), "it stays off");
    view.update(cx, |v, cx| v.debug_permissions("hello", cx));
    frame(cx);
    view.update(cx, |v, cx| v.debug_permissions_on(cx));
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension_on("hello")));

    // 0.2.0 by the same key, asking to notify: its permission change is
    // shown, and after the update it isn't granted until allowed.
    let v2 = hello_package(t.path(), "0.2.0", r#", "notify": "Tell you when it's done""#, Some(&author));
    let (note, id) = look(&view, &v2, cx);
    assert_eq!((note, id.as_deref()), (None, Some("hello")));
    assert!(cx.debug_bounds("extension-add-file-permission-notify").is_some(), "the new permission is shown: {:?} {:?}", cx.debug_bounds("extension-add-file-card"), cx.debug_bounds("extension-add-file-permissions"));
    view.update(cx, |v, cx| v.install_from_package(cx));
    until(cx, "the update", |cx| view.read_with(cx, |v, _| v.add_file_state().is_none()));
    assert_eq!(install::local(&exts.join("hello")).version.as_deref(), Some("0.2.0"));
    assert_eq!(install::previous(&exts, "hello").and_then(|p| p.version).as_deref(), Some("0.1.0"), "kept for Go back");
    let (granted, undecided) = cx.update(|_, cx| {
        let m = crate::extensions::manifest_of("hello", cx).unwrap();
        let d = crate::ui_state::get(cx).extension("hello").grants;
        (crate::tiles::grants_of(&m, cx).has("notify"), permissions::undecided(&m, &d).iter().map(|p| p.name.clone()).collect::<Vec<_>>())
    });
    assert!(!granted && undecided == ["notify"], "a new permission waits for Allow: {undecided:?}");
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension_on("hello")), "on again after the update");

    // 0.3.0 by another key: refused until "Trust the new key".
    let other = Signing::new();
    let v3 = hello_package(t.path(), "0.3.0", "", Some(&other));
    let (note, _) = look(&view, &v3, cx);
    assert_eq!(note, None);
    assert!(cx.debug_bounds("extension-add-file-new-key-hello").is_some(), "the new key's switch");
    view.update(cx, |v, cx| v.install_from_package(cx));
    frame(cx);
    let (_, note, _) = view.read_with(cx, |v, _| v.add_file_state()).unwrap();
    assert!(note.as_deref().is_some_and(|n| n.contains("another key")), "{note:?}");
    assert_eq!(install::local(&exts.join("hello")).version.as_deref(), Some("0.2.0"), "not installed");
    view.update(cx, |v, cx| v.trust_new_key(true, cx));
    view.update(cx, |v, cx| v.install_from_package(cx));
    until(cx, "the update", |cx| view.read_with(cx, |v, _| v.add_file_state().is_none()));
    assert_eq!(install::local(&exts.join("hello")).version.as_deref(), Some("0.3.0"));
    assert_eq!(signer_of(&exts), Some(other.signer()), "the new key is trusted from now on");

    // An older file: refused, pointing at Go back.
    let (note, _) = look(&view, &v1, cx);
    assert!(note.as_deref().is_some_and(|n| n.contains("Go back")), "{note:?}");
    view.update(cx, |v, cx| v.close_add_file(cx));

    // Go back: 0.2.0 with the first key.
    view.update(cx, |v, cx| v.start_back("hello", false, cx));
    until(cx, "the way back", |_| install::local(&exts.join("hello")).version.as_deref() == Some("0.2.0"));
    assert_eq!(signer_of(&exts), Some(author.signer()));

    // Remove: its data asked about (kept unless ticked), then deleted.
    let data = root.join("extension-data/hello");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("storage.json"), "{\"n\": 1}").unwrap();
    view.update(cx, |v, cx| {
        v.exts.open.insert("hello".into(), true);
        v.exts.confirm_remove = Some("hello".into());
        cx.notify();
    });
    frame(cx);
    assert!(cx.debug_bounds("extension-remove-data-hello").is_some(), "its data is asked about");
    view.update(cx, |v, cx| {
        v.exts.remove_data.insert("hello".into());
        let m = crate::extensions::manifest_of("hello", cx).unwrap();
        v.start_remove(&m, cx);
    });
    until(cx, "the removal", |_| !exts.join("hello").exists());
    until(cx, "its data gone", |_| !data.exists());
    assert!(install::previous(&exts, "hello").is_none(), "the kept copy went too");
}

/// `pitwall extension install <file>` as the CLI sends it: refused at
/// once when unsigned without `unsigned`; signed, the approval names the
/// key and installs it on yes.
#[gpui::test]
fn the_cli_installs_a_package_through_the_approval(cx: &mut TestAppContext) {
    use pitwall_core::testing::{ManualClock, MemStore, RecordingSink};
    use pitwall_proto::ApprovalAnswer;
    use serde_json::{json, Value};
    let t = TempDir::new("pkg-cli");
    let root = t.path().join("data");
    let exts = root.join("extensions");
    std::fs::create_dir_all(&exts).unwrap();
    init(&root, cx);
    let (_, cx) = cx.add_window_view(|_, _| gpui::Empty);
    let engine = pitwall_core::Engine::open(pitwall_core::Deps { paths: pitwall_core::paths::Paths::new(t.path().join("engine")), events: RecordingSink::new(), clock: ManualClock::new(1), store: MemStore::with(vec![]), providers: vec![] });
    let approvals = pitwall_daemon::Approvals::new(Duration::from_secs(60));
    let backend = crate::extensions::agent_setup::AppExtensions::new();
    let sock_dir = std::env::temp_dir().join(format!("pwpkg-{}", std::process::id()));
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket = sock_dir.join("d.sock");
    let cfg = pitwall_daemon::Config { socket: socket.clone(), version: "test".into(), settings: None, workspace: None, viewer: None, events: None, extensions: None, ext_backend: Some(backend.clone()) };
    let identify = pitwall_daemon::ProcessIdentity::with_extensions(engine.clone(), pitwall_daemon::Registry::new());
    let server = pitwall_daemon::serve(engine, approvals.clone(), identify, cfg).unwrap();
    cx.update(|_, cx| crate::extensions::agent_setup::attach(backend, cx));
    let act = |params: Value| {
        let socket = socket.clone();
        std::thread::spawn(move || -> Result<Value, pitwall_client::Error> {
            let mut c = pitwall_client::Client::connect(&socket)?;
            c.call("extension.act", params)
        })
    };
    let finish = |cx: &mut VisualTestContext, h: std::thread::JoinHandle<Result<Value, pitwall_client::Error>>| {
        until(cx, "the answer", |_| h.is_finished());
        h.join().unwrap()
    };
    let bare = t.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    let unsigned = hello_package(&bare, "0.1.0", "", None);
    match finish(cx, act(json!({ "id": "hello", "op": "installPackage", "path": unsigned }))) {
        Err(pitwall_client::Error::Server(e)) => assert!(e.message.contains("isn't signed") && e.message.contains("--unsigned"), "{}", e.message),
        other => panic!("not refused: {other:?}"),
    }
    assert!(approvals.pending().is_empty(), "nothing asked");
    let author = Signing::new();
    let signed = hello_package(t.path(), "0.1.0", "", Some(&author));
    let h = act(json!({ "id": "hello", "op": "installPackage", "path": signed }));
    until(cx, "the approval", |_| !approvals.pending().is_empty());
    let p = approvals.pending()[0].clone();
    assert!(p.summary.contains("from a signed package"), "{}", p.summary);
    assert!(p.details.iter().any(|d| d.contains(&author.signer().key_id) && d.contains("trusts this key")), "{:?}", p.details);
    approvals.answer(&ApprovalAnswer { id: p.id.clone(), allow: true, remember: false }).unwrap();
    let done = finish(cx, h).unwrap();
    assert!(done["text"].as_str().is_some_and(|t| t.starts_with("Installed")), "{done}");
    assert_eq!(signer_of(&exts), Some(author.signer()));
    server.stop();
    let _ = std::fs::remove_dir_all(&sock_dir);
}
