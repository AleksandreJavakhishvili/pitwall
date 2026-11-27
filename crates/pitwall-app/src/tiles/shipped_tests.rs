//! The tiles of the extensions shipped in this repository, in the app
//! itself, headless (GPUI's test platform, no window shown): for every
//! `extensions/*/test/app.json`, the extension is installed from its
//! folder (its committed tile.wasm), its settings and secrets set, turned
//! on with what it asks for, and its tile opened against a fake server on
//! this computer that answers from the fixture's table, through the app's
//! own `net`. Each step clicks a node and waits for texts.
//!
//! The fixture:
//!
//! - `tile`: the tile's id.
//! - `without`: manifest keys left out of the installed copy, each with
//!   why (a `service`, so that nothing real is started).
//! - `settings`, `secrets`: values to set; `{url}` is the fake's address.
//! - `server`: `{method, path, status?, body?, hold?}`, matched in order on
//!   the method and the path without its query; `hold` keeps the request
//!   open (a long poll with no news). Anything else is a 404.
//! - `size`: the view's width and height.
//! - `steps`: `{click?, see, shot?}`.
//!
//! With `PITWALL_TILE_SHOTS=<folder>`, each step's `shot` is saved there,
//! dark and light, as `<extension>-<shot>-<theme>.png`.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;
use pitwall_tile_host::ui;
use serde_json::Value;

use crate::extensions::catalog;
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(120);

fn extensions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions")
}

/// A fake HTTP server on 127.0.0.1 answering from a table; held requests
/// stay open until it is dropped.
struct Fake {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    fn start(table: Vec<Value>) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (table, seen2) = (Arc::new(table), seen.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (table, seen) = (table.clone(), seen2.clone());
                std::thread::spawn(move || answer(stream, &table, &seen));
            }
        });
        Fake { url, seen }
    }
}

fn answer(mut stream: TcpStream, table: &[Value], seen: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let path = target.split('?').next().unwrap_or("").to_string();
    seen.lock().unwrap().push(format!("{method} {path}"));
    let row = table.iter().find(|r| r["method"].as_str() == Some(method.as_str()) && r["path"].as_str() == Some(path.as_str()));
    let Some(row) = row else { return reply(&mut stream, 404, &serde_json::json!({ "error": { "code": "not_found", "message": "not found" } })) };
    if row["hold"].as_bool() == Some(true) {
        // Open until the other side gives up or the test ends.
        let _ = reader.read_to_end(&mut Vec::new());
        return;
    }
    reply(&mut stream, row["status"].as_u64().unwrap_or(200) as u16, &row["body"]);
}

fn reply(stream: &mut TcpStream, status: u16, body: &Value) {
    let body = serde_json::to_vec(body).unwrap();
    let head = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

fn with_url(v: &Value, url: &str) -> Option<String> {
    v.as_str().map(|s| s.replace("{url}", url))
}

#[test]
fn shipped_tiles_work_in_the_app() {
    let mut ran = 0;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(extensions_dir()).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("test/app.json").is_file()).collect();
    dirs.sort();
    for dir in dirs {
        run(&dir);
        ran += 1;
    }
    assert!(ran > 0, "no extension under extensions/ has a test/app.json");
}

fn run(dir: &Path) {
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("test/app.json")).unwrap()).unwrap();
    let mut manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("extension.json")).unwrap()).unwrap();
    let id = manifest["id"].as_str().unwrap().to_string();
    for key in fixture["without"].as_object().into_iter().flat_map(|o| o.keys()) {
        manifest.as_object_mut().unwrap().remove(key);
    }
    let tile_id = fixture["tile"].as_str().unwrap().to_string();
    let fake = Fake::start(fixture["server"].as_array().cloned().unwrap_or_default());
    let shots = std::env::var("PITWALL_TILE_SHOTS").ok().map(PathBuf::from);
    if let Some(s) = &shots {
        std::fs::create_dir_all(s).unwrap();
    }

    // Its folder as shipped, but for what the fixture leaves out, in the
    // extensions folder (as installed).
    let t = TempDir::new("shipped-tiles");
    let root = t.path().join("data");
    let src = pitwall_core::paths::Paths::new(root.clone()).extensions_dir().join(&id);
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("extension.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    for tile in manifest["contributes"]["tiles"].as_array().into_iter().flatten() {
        let wasm = tile["wasm"].as_str().unwrap();
        std::fs::copy(dir.join(wasm), src.join(wasm)).unwrap_or_else(|e| panic!("{id}: its {wasm}: {e}"));
    }

    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let r2 = root.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2), cx);
        catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(root.clone()));
        let paths = pitwall_core::paths::Paths::new(root.clone());
        crate::accounts::model(cx).update(cx, |m, _| m.set_paths(paths));
        crate::settings::data::set_page(cx, Page::Extensions);
    });
    let settle = |cx: &mut HeadlessAppContext, what: &str, done: &mut dyn FnMut(&mut gpui::App) -> bool| {
        let end = Instant::now() + WAIT;
        loop {
            cx.run_until_parked();
            if cx.update(|cx| done(cx)) {
                return;
            }
            assert!(Instant::now() < end, "{what}: timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        let Some(out) = &shots else { return };
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            cx.run_until_parked();
            let _ = cx.update_window(w, |_, window, _| window.refresh());
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("{id}-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };

    // Set up, and turned on.
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let _shell = cx
        .open_window(size(px(900.), px(730.)), move |_, cx| {
            let v = cx.new(|cx| SettingsView::new(true, cx));
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), false))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    let found = cx.update(|cx| crate::extensions::found(cx).iter().map(|f| f.manifest.as_ref().map(|m| m.id.clone()).map_err(Clone::clone)).collect::<Vec<_>>());
    assert!(found.iter().any(|f| f.as_deref() == Ok(id.as_str())), "{id}: not found: {found:?}");
    let errors = Arc::new(Mutex::new(Vec::new()));
    cx.update(|cx| {
        for (k, v) in fixture["settings"].as_object().into_iter().flatten() {
            crate::extensions::connections::set_setting_value(&id, k, with_url(v, &fake.url), cx).unwrap();
        }
        for (k, v) in fixture["secrets"].as_object().into_iter().flatten() {
            let e = errors.clone();
            crate::extensions::set_secret(&id, k, with_url(v, &fake.url).unwrap(), cx, move |err, _| e.lock().unwrap().extend(err));
        }
    });
    cx.run_until_parked();
    assert!(errors.lock().unwrap().is_empty(), "{id}: {:?}", errors.lock().unwrap());
    let i2 = id.clone();
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions(&i2, cx)));
    cx.run_until_parked();
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions_on(cx)));
    cx.run_until_parked();
    settle(&mut cx, "turned on", &mut |cx| crate::ui_state::get(cx).extension_on(&id));

    // Its tile.
    let (w, h) = (fixture["size"][0].as_f64().unwrap_or(900.) as f32, fixture["size"][1].as_f64().unwrap_or(560.) as f32);
    let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vt2 = vt.clone();
    let key = format!("{id}/{tile_id}");
    let win = cx
        .open_window(size(px(w), px(h)), move |_, cx| {
            let v = crate::main_screen::view_tile::view(&key, cx);
            *vt2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), true))
        })
        .unwrap();
    let vtile: Entity<crate::main_screen::view_tile::ViewTile> = vt.borrow().clone().unwrap();
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    for (n, step) in fixture["steps"].as_array().into_iter().flatten().enumerate() {
        if let Some(path) = step["click"].as_str() {
            let ok = cx.update(|cx| tile.read(cx).click(path, ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 1., y: 1. }, cx));
            assert!(ok, "{id}, step {}: no node {path} to click; it shows {:?}", n + 1, cx.update(|cx| tile.read(cx).texts()));
        }
        let want: Vec<String> = step["see"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect();
        let what = format!("{id}, step {}: {want:?} (requests: {:?})", n + 1, fake.seen.lock().unwrap());
        settle(&mut cx, &what, &mut |cx| {
            let texts = tile.read(cx).texts();
            want.iter().all(|w| texts.iter().any(|t| t.contains(w.as_str())))
        });
        if let Some(name) = step["shot"].as_str() {
            shot(&mut cx, win.into(), name);
        }
    }
    let _ = cx.update_window(win.into(), |_, window, _| window.remove_window());
    cx.run_until_parked();
}
