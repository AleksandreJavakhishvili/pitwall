//! The index tool against a local fixture index repository: nothing
//! reaches the network or GitHub.

use pitwall_core::extensions::index::{self, repo, FixtureSource, IndexSource};
use pitwall_core::testing::index::IndexFixture;
use pitwall_core::testing::Signing;

const NOW: &str = "2026-11-27T10:00:00Z";

fn hello_wasm() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../pitwall-tile-host/tests/fixtures/hello/tile.wasm")).expect("the hello fixture")
}

fn secret(s: &Signing, dir: &std::path::Path) -> minisign::SecretKey {
    let path = dir.join("index.key");
    std::fs::write(&path, s.secret_key_file()).unwrap();
    pitwall_index::secret_key(&path, "UNSET_FOR_TESTS").unwrap()
}

#[test]
fn a_real_tile_passes_the_check_and_starts_headless() {
    let f = IndexFixture::new();
    f.release_with("hello", "0.1.0", "", &hello_wasm(), &f.author);
    let r = pitwall_index::check(&f.root(), None, "octo", None, f.http.as_ref(), true, NOW);
    assert!(r.passed(), "{}", r.markdown());
    assert_eq!(r.checked, vec![("hello".to_string(), "0.1.0".to_string())]);
    assert!(r.markdown().contains("its tiles start headless"));
}

#[test]
fn a_package_that_isnt_a_working_component_fails_the_headless_run() {
    let f = IndexFixture::new();
    f.release("broken", "1.0.0", "");
    let r = pitwall_index::check(&f.root(), None, "octo", None, f.http.as_ref(), true, NOW);
    assert!(!r.passed());
    assert!(r.problems.iter().any(|p| p.contains("didn't start headless") || p.contains("stopped")), "{:?}", r.problems);
}

#[test]
fn an_update_by_its_author_with_the_same_permissions_can_be_merged_by_the_bot() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", "");
    let base = tempfile::tempdir().unwrap();
    copy(&f.root(), base.path());
    f.release("ci-status", "1.1.0", "");
    let r = pitwall_index::check(&f.root(), Some(base.path()), "octo", None, f.http.as_ref(), false, NOW);
    assert!(r.auto_merge(), "{}", r.markdown());
    // A new host: a maintainer merges.
    f.release("ci-status", "1.2.0", r#""net": {"reason": "Runs", "hosts": ["api.github.com"]}"#);
    let r = pitwall_index::check(&f.root(), Some(base.path()), "octo", None, f.http.as_ref(), false, NOW);
    assert!(r.passed() && !r.auto_merge(), "{}", r.markdown());
    assert!(r.markdown().contains("permission net changes"));
}

#[test]
fn build_signs_what_pitwall_reads_and_review_makes_a_version_verified() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", "");
    let keys = tempfile::tempdir().unwrap();
    let review_key = secret(&f.reviewer, keys.path());
    pitwall_index::review(&f.root(), "ci-status", "1.0.0", "max", &review_key, "2026-11-27", true).unwrap();
    // Not max's key: refused.
    assert!(pitwall_index::review(&f.root(), "ci-status", "1.0.0", "max", &secret(&Signing::new(), keys.path()), "2026-11-27", true).is_err());
    assert!(pitwall_index::review(&f.root(), "ci-status", "1.0.0", "max", &review_key, "2026-11-27", false).is_err());

    let out = f.dist();
    let w = pitwall_index::build(&f.root(), &out, &secret(&f.index_key, keys.path()), f.http.as_ref(), NOW, false).unwrap();
    assert_eq!(w.extensions, 1);
    assert_eq!(w.screens, 1);
    let got = FixtureSource::dir(&out, &[f.index_key.signer()]).fetch().unwrap();
    let l = got.index.listing("ci-status").unwrap();
    assert!(l.is_verified(l.release("1.0.0").unwrap(), &[f.reviewer.signer()]));
    // A second build downloads nothing it has.
    let before = f.http.requests().len();
    pitwall_index::build(&f.root(), &out, &secret(&f.index_key, keys.path()), f.http.as_ref(), "2026-11-28T10:00:00Z", false).unwrap();
    assert_eq!(f.http.requests().len(), before);
    // Signed by another key: Pitwall refuses it.
    assert!(FixtureSource::dir(&out, &[Signing::new().signer()]).fetch().is_err());
}

#[test]
fn the_fixture_repositorys_categories_are_the_tools() {
    let f = IndexFixture::new();
    assert_eq!(std::fs::read_to_string(f.root().join("categories.toml")).unwrap(), repo::categories_toml());
    assert_eq!(index::CATEGORIES.len(), 9);
    let template = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/template/categories.toml")).unwrap();
    assert_eq!(template, repo::categories_toml(), "template/categories.toml: run `pitwall-index categories > template/categories.toml`");
    // The template is a repository the tool loads.
    assert!(repo::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/template"))).is_ok());
}

fn copy(from: &std::path::Path, to: &std::path::Path) {
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&t).unwrap();
            copy(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}
