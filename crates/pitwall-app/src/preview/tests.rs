use std::path::Path;

use gpui::{bounds, point, px, size, Bounds, Pixels};

use super::*;
use crate::webview::Snapshot;

fn urls(u: &[&str]) -> Vec<String> {
    u.iter().map(|s| s.to_string()).collect()
}

const VITE: &str = "http://localhost:5173/";

#[test]
fn the_page_follows_its_terminal() {
    let mut f = Follow::new(VITE, &urls(&[VITE]), 1);
    assert_eq!(f.update(&urls(&[VITE]), 1, true), None, "nothing new");
    // The server restarted: its address printed again.
    assert_eq!(f.update(&urls(&[VITE]), 2, true), Some(Act::Reload));
    assert_eq!(f.update(&urls(&[VITE]), 2, true), None);
    // It came back on another port.
    let next = "http://localhost:5174/";
    assert_eq!(f.update(&urls(&[next, VITE]), 3, true), Some(Act::Navigate(next.into())));
    assert_eq!(f.url, next);
    // The agent stopped: once.
    assert_eq!(f.update(&[], 0, false), Some(Act::Stop));
    assert_eq!(f.server, Server::Stopped);
    assert_eq!(f.update(&[], 0, false), None);
    // Running again, the address not printed yet: still stopped.
    assert_eq!(f.update(&[], 0, true), None);
    // Printed again (a new terminal counts from 0): back on it.
    assert_eq!(f.update(&urls(&[next]), 1, true), Some(Act::Navigate(next.into())));
    assert_eq!(f.server, Server::Live);
}

#[test]
fn a_picked_address_stays_until_the_terminal_prints_a_new_one() {
    let other = "http://localhost:3000";
    let mut f = Follow::new(other, &urls(&[VITE, other]), 2);
    assert_eq!(f.update(&urls(&[VITE, other]), 2, true), None, "not moved to the latest");
    assert_eq!(f.url, other);
    let next = "http://localhost:8080";
    assert_eq!(f.update(&urls(&[next, VITE, other]), 3, true), Some(Act::Navigate(next.into())));
}

#[test]
fn a_failed_load_waits_for_the_server() {
    let mut f = Follow::new(VITE, &urls(&[VITE]), 1);
    f.failed("Could not connect to the server.");
    assert!(matches!(f.server, Server::Unreachable(_)));
    assert_eq!(f.update(&urls(&[VITE]), 1, true), None);
    // The server printed its address again: try.
    assert_eq!(f.update(&urls(&[VITE]), 2, true), Some(Act::Reload));
    assert_eq!(f.server, Server::Live);
    f.failed("down");
    f.retry();
    assert_eq!(f.server, Server::Live, "the user's reload tries again");
    // A stopped server doesn't turn into a failed one.
    f.update(&[], 0, false);
    f.failed("down");
    assert_eq!(f.server, Server::Stopped);
}

fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    bounds(point(px(x), px(y)), size(px(w), px(h)))
}

#[test]
fn the_view_moves_only_when_its_place_changes() {
    let mut s = Stage::default();
    let at = b(100., 50., 400., 300.);
    assert_eq!(s.frame(Some(at), false, true), vec![StageOp::SetBounds(at), StageOp::Show]);
    assert!(s.frame(Some(at), false, true).is_empty(), "same place: nothing");
    let moved = b(120., 50., 380., 300.);
    assert_eq!(s.frame(Some(moved), false, true), vec![StageOp::SetBounds(moved)]);
    // Not drawn (another space): hidden, then idle.
    assert_eq!(s.frame(None, false, true), vec![StageOp::Hide]);
    assert!(s.idle());
    assert!(s.frame(None, false, true).is_empty());
    // A zero-size box counts as not drawn.
    assert!(s.frame(Some(b(0., 0., 0., 10.)), false, true).is_empty());
    assert_eq!(s.frame(Some(moved), false, true), vec![StageOp::Show]);
    assert!(!s.idle());
}

#[test]
fn a_cover_swaps_the_view_for_its_snapshot() {
    let mut s = Stage::default();
    let at = b(0., 0., 100., 100.);
    s.frame(Some(at), false, true);
    // A menu opens over it: a snapshot first, the view stays meanwhile.
    assert_eq!(s.frame(Some(at), true, true), vec![StageOp::Snapshot]);
    assert!(s.visible && s.picture().is_none());
    assert!(s.frame(Some(at), true, true).is_empty(), "asked once");
    let image = to_image(Snapshot { width: 1, height: 1, rgba: vec![1, 2, 3, 4] });
    assert_eq!(s.snapshot(image), vec![StageOp::Hide]);
    assert!(matches!(s.picture(), Some(Some(_))));
    assert!(!s.idle(), "covered is not idle: no drop timer");
    // The menu closes: the view again, the picture gone.
    assert_eq!(s.frame(Some(at), false, true), vec![StageOp::Show]);
    assert!(s.picture().is_none());
}

#[test]
fn a_snapshot_after_the_cover_left_is_dropped() {
    let mut s = Stage::default();
    let at = b(0., 0., 100., 100.);
    s.frame(Some(at), false, true);
    s.frame(Some(at), true, true);
    assert!(s.frame(Some(at), false, true).is_empty(), "still showing");
    assert!(s.snapshot(None).is_empty());
    assert!(s.visible && s.picture().is_none());
}

#[test]
fn an_engine_without_snapshots_still_steps_aside() {
    let mut s = Stage::default();
    let at = b(0., 0., 100., 100.);
    s.frame(Some(at), false, true);
    s.frame(Some(at), true, true);
    assert_eq!(s.snapshot(None), vec![StageOp::Hide]);
    assert!(matches!(s.picture(), Some(None)), "the pane's background in its place");
}

#[test]
fn a_stopped_server_hides_the_view() {
    let mut s = Stage::default();
    let at = b(0., 0., 100., 100.);
    s.frame(Some(at), false, true);
    assert_eq!(s.frame(Some(at), false, false), vec![StageOp::Hide]);
    assert!(s.idle());
    // Covered while stopped: nothing to swap.
    assert!(s.frame(Some(at), true, false).is_empty());
    assert_eq!(s.frame(Some(at), false, true), vec![StageOp::Show]);
}

#[test]
fn what_is_drawn_after_the_view_covers_it() {
    // (id, bounds) in paint order.
    let list = [
        (1, b(0., 0., 1000., 800.)), // the screen, drawn first
        (2, b(100., 100., 400., 300.)), // the view
        (3, b(600., 100., 100., 20.)), // a button beside it
    ];
    let place = |id: i32, tip| placed_by(&list, |h| h.0 == id, |h| h.1, tip);
    assert_eq!(place(2, None), Some((b(100., 100., 400., 300.), false)));
    assert_eq!(place(9, None), None, "not drawn");
    // A tooltip over it.
    assert_eq!(place(2, Some(b(450., 350., 100., 30.))).map(|p| p.1), Some(true));
    // Touching edges don't count.
    assert_eq!(place(2, Some(b(500., 100., 20., 20.))).map(|p| p.1), Some(false));
    let menu = [(2, b(100., 100., 400., 300.)), (4, b(300., 200., 160., 200.))];
    assert_eq!(placed_by(&menu, |h| h.0 == 2, |h| h.1, None).map(|p| p.1), Some(true));
}

#[test]
fn snapshots_become_bgra_images() {
    let img = to_image(Snapshot { width: 2, height: 1, rgba: vec![10, 20, 30, 255, 40, 50, 60, 255] })
        .expect("an image");
    assert_eq!(img.as_bytes(0), Some(&[30u8, 20, 10, 255, 60, 50, 40, 255][..]));
    assert!(to_image(Snapshot { width: 2, height: 2, rgba: vec![0; 4] }).is_none(), "short data");
    assert!(to_image(Snapshot { width: 0, height: 0, rgba: vec![] }).is_none());
}

#[test]
fn each_project_has_its_own_storage() {
    let root = Path::new("/data");
    let a = data_dir(root, "/work/my app");
    assert!(a.starts_with("/data/webview"));
    assert!(a.file_name().unwrap().to_string_lossy().starts_with("my_app-"));
    assert_eq!(a, data_dir(root, "/work/my app"), "stable");
    assert_ne!(a, data_dir(root, "/other/my app"));
}
