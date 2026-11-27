use super::repo;
use super::*;
use crate::testing::index::{IndexFixture, BASE};
use crate::testing::Signing;

const NOW: &str = "2026-11-27T10:00:00Z";
const PERMS: &str = r#""net": {"reason": "To read runs", "hosts": ["api.github.com"]}"#;

fn judge<'a>(f: &'a IndexFixture, blocked: &'a Blocked, keys: &'a [Signer]) -> Judge<'a> {
    let _ = f;
    Judge { review_keys: keys, blocked, pitwall: "0.14.0", worlds: &["pitwall:tile@0.1.0"] }
}

#[test]
fn a_repository_builds_into_an_index_that_reads_back() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    let b = f.publish(NOW);
    let l = b.index.listing("ci-status").unwrap();
    assert_eq!(l.name, "ci-status tile");
    assert_eq!(l.github, "octo");
    assert_eq!(l.permissions, vec![ListedPermission { name: "net".into(), reason: "To read runs".into(), hosts: vec!["api.github.com".into()], amount: None }]);
    assert_eq!(l.versions[0].date, "2026-11-27");
    assert_eq!(l.versions[0].changelog.as_deref(), Some("- Made for a test."));
    assert_eq!(l.screens.len(), 1);
    assert!(l.screens[0].starts_with("screens/ci-status/"));
    assert_eq!(b.screens.len(), 1);
    // Signed and read back as Pitwall reads it.
    let got = f.source().fetch().unwrap();
    assert_eq!(got.index, b.index);
    assert_eq!(got.origin, Origin::Fixture);
}

#[test]
fn a_build_refuses_a_package_that_isnt_the_one_listed_or_signed_by_the_author() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    f.edit("ci-status", |e| e.versions[0].sha256 = "a".repeat(64));
    let err = f.build(NOW, None).unwrap_err().join("\n");
    assert!(err.contains("not the aaaa"), "{err}");

    let g = IndexFixture::new();
    g.release_signed("ci-status", "1.0.0", PERMS, &Signing::new());
    let err = g.build(NOW, None).unwrap_err().join("\n");
    assert!(err.contains("didn't check against the author's keys"), "{err}");
}

#[test]
fn the_repository_is_checked_offline() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    f.edit("ci-status", |e| {
        e.categories = vec!["weather".into()];
        e.versions[0].url = "https://example.com/x.pwtile".into();
        e.author = "nobody".into();
    });
    let p = repo::load(&f.root()).unwrap_err().join("\n");
    assert!(p.contains("“weather” isn't one of the categories"), "{p}");
    assert!(p.contains("isn't a release asset of https://github.com/octo/pitwall-ci-status"), "{p}");
    assert!(p.contains("has no authors/nobody.toml"), "{p}");
    // A key shared by two authors.
    let g = IndexFixture::new();
    g.write_author("eve", "eve", &[g.author.signer().public_key]);
    assert!(repo::load(&g.root()).unwrap_err().join("\n").contains("is also"));
}

#[test]
fn a_review_must_be_a_maintainers_signature_over_the_version() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    f.review("ci-status", "1.0.0");
    assert!(repo::load(&f.root()).is_ok());
    // Moved onto another version: no longer checks.
    f.release("ci-status", "1.1.0", PERMS);
    f.edit("ci-status", |e| e.versions[1].review = e.versions[0].review.clone());
    let p = repo::load(&f.root()).unwrap_err().join("\n");
    assert!(p.contains("its review's signature isn't max's"), "{p}");
}

#[test]
fn browse_offers_the_last_verified_version_by_default() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    f.review("ci-status", "1.0.0");
    f.release("ci-status", "1.1.0", PERMS);
    let b = f.publish(NOW);
    let keys = [f.reviewer.signer()];
    let j = judge(&f, &b.blocked, &keys);
    let l = b.index.listing("ci-status").unwrap();
    assert_eq!(l.default_release(&j).unwrap().version, "1.0.0");
    assert_eq!(l.newest_release(&j).unwrap().version, "1.1.0");
    assert_eq!(l.standing(l.release("1.0.0").unwrap(), &j), Standing::Verified);
    assert_eq!(l.standing(l.release("1.1.0").unwrap(), &j), Standing::NotYetReviewed);
    // Without the review key (or with the index's key), nothing is Verified.
    let none: [Signer; 0] = [];
    let j2 = judge(&f, &b.blocked, &none);
    assert_eq!(l.default_release(&j2).unwrap().version, "1.1.0");
    assert_eq!(l.standing(l.release("1.0.0").unwrap(), &j2), Standing::NotVerified);
    let wrong = [f.index_key.signer()];
    assert!(!l.is_verified(l.release("1.0.0").unwrap(), &wrong));
}

#[test]
fn yanked_and_blocked_versions_are_refused_with_the_reason() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    f.release("ci-status", "1.1.0", PERMS);
    f.yank("ci-status", "1.1.0");
    let sha = f.sha256("ci-status", "1.0.0");
    f.block(Block { id: "ci-status".into(), versions: vec![], sha256: vec![sha.clone()], reason: "it sends your tokens away".into(), date: "2026-11-27".into() });
    let b = f.publish(NOW);
    let keys: [Signer; 0] = [];
    let j = judge(&f, &b.blocked, &keys);
    let l = b.index.listing("ci-status").unwrap();
    assert_eq!(l.default_release(&j), None);
    assert_eq!(l.standing(l.release("1.0.0").unwrap(), &j), Standing::Blocked("it sends your tokens away".into()));
    assert_eq!(l.standing(l.release("1.1.0").unwrap(), &j), Standing::Yanked);
    let r = refusal(&b.index, &b.blocked, "ci-status", "1.0.0", Some(&sha)).unwrap();
    assert_eq!(r, "ci-status 1.0.0 is blocked by the extensions index: it sends your tokens away. Pitwall won't install it.");
    let r = refusal(&b.index, &b.blocked, "ci-status", "1.1.0", None).unwrap();
    assert!(r.contains("was withdrawn by its author"), "{r}");
    let off = b.blocked.turned_off(&[("ci-status".into(), "1.0.0".into(), Some(sha.clone())), ("notes".into(), "1.0.0".into(), None)]);
    assert_eq!(off.len(), 1);
    assert!(installed_note(&b.index, &b.blocked, "ci-status", "1.0.0", Some(&sha)).unwrap().starts_with("Turned off"));
}

#[test]
fn a_delisted_extension_leaves_browse_and_says_why() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    f.edit("ci-status", |e| e.delisted = Some("unmaintained".into()));
    let b = f.publish(NOW);
    assert!(b.index.listing("ci-status").is_none());
    let note = installed_note(&b.index, &b.blocked, "ci-status", "1.0.0", None).unwrap();
    assert_eq!(note, "No longer listed (unmaintained). It keeps working, but gets no updates.");
    assert!(refusal(&b.index, &b.blocked, "ci-status", "1.0.0", None).unwrap().contains("no longer listed"));
    assert!(refusal(&b.index, &b.blocked, "other", "1.0.0", None).unwrap().contains("has no extension"));
}

#[test]
fn a_new_version_keeps_what_was_recorded_and_says_what_it_changes() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    let first = f.build("2026-11-01T00:00:00Z", None).unwrap();
    f.release("ci-status", "1.1.0", r#""net": {"reason": "To read runs", "hosts": ["api.github.com", "evil.example"]}"#);
    let b = f.build(NOW, Some(&first.index)).unwrap();
    let l = b.index.listing("ci-status").unwrap();
    assert_eq!(l.release("1.0.0").unwrap().date, "2026-11-01");
    assert_eq!(l.release("1.1.0").unwrap().date, "2026-11-27");
    assert_eq!(b.sensitive, vec![("ci-status".into(), "1.1.0".into(), "permission net changes".into())]);
    assert_eq!(l.permissions[0].hosts, vec!["api.github.com", "evil.example"]);
    // 1.0.0 was only downloaded to compare with.
    assert_eq!(b.checked.iter().map(|c| c.version.as_str()).collect::<Vec<_>>(), vec!["1.1.0"]);
}

#[test]
fn a_new_key_on_a_version_is_sensitive() {
    let f = IndexFixture::new();
    let other = Signing::new();
    f.write_author("octo", "octo", &[f.author.signer().public_key, other.signer().public_key]);
    f.release("ci-status", "1.0.0", PERMS);
    f.release_signed("ci-status", "1.1.0", PERMS, &other);
    let b = f.build(NOW, None).unwrap();
    assert!(b.sensitive.iter().any(|(_, v, w)| v == "1.1.0" && w.starts_with("signed by key")), "{:?}", b.sensitive);
}

#[test]
fn a_pull_request_is_judged_against_its_base() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    let base = repo::load(&f.root()).unwrap();
    // A plain update by its author: mergeable by the bot.
    f.release("ci-status", "1.1.0", PERMS);
    let head = repo::load(&f.root()).unwrap();
    let c = repo::judge(&base, &head, "octo");
    assert!(c.auto_mergeable(), "{c:?}");
    assert_eq!(c.new_versions, vec![("ci-status".into(), "1.1.0".into())]);
    // By someone else: a maintainer looks.
    assert!(!repo::judge(&base, &head, "mallory").auto_mergeable());
    // A changed listed version: refused.
    f.edit("ci-status", |e| e.versions[0].sha256 = "b".repeat(64));
    let c = repo::judge(&base, &repo::load(&f.root()).unwrap(), "octo");
    assert!(c.problems.iter().any(|p| p.contains("is never changed")), "{c:?}");
    // A look-alike new id needs a maintainer.
    f.edit("ci-status", |e| e.versions[0].sha256 = base.entries["ci-status"].versions[0].sha256.clone());
    f.release("ci-statu5", "1.0.0", PERMS);
    let c = repo::judge(&base, &repo::load(&f.root()).unwrap(), "octo");
    assert!(c.needs_maintainer.iter().any(|p| p.contains("looks like ci-status")), "{c:?}");
}

#[test]
fn look_alikes_catch_typosquats() {
    let ids = ["ci-status", "notes", "db-browser"];
    assert_eq!(repo::look_alikes("cistatus", ids), vec!["ci-status"]);
    assert_eq!(repo::look_alikes("n0tes", ids), vec!["notes"]);
    assert_eq!(repo::look_alikes("db-browsers", ids), vec!["db-browser"]);
    assert!(repo::look_alikes("weather", ids).is_empty());
}

#[test]
fn the_https_source_caches_checks_and_falls_back() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    let b = f.publish(NOW);
    let cache = f.dir.path().join("cache");
    let s = HttpsSource::new(f.http.clone(), f.config(), &cache);
    let got = s.fetch().unwrap();
    assert_eq!(got.origin, Origin::Network);
    assert_eq!(got.index, b.index);
    // Second fetch: conditional, not modified.
    assert_eq!(s.fetch().unwrap().origin, Origin::NotModified);
    assert!(f.http.requests().iter().any(|(u, e)| u == &format!("{BASE}index.json") && e.is_some()));
    // Offline: the cached copy, saying why.
    f.http.set_offline(true);
    let got = s.fetch().unwrap();
    assert!(matches!(got.origin, Origin::Cache { .. }));
    assert_eq!(got.index, b.index);
    f.http.set_offline(false);
    // A tampered index: refused, the cache kept.
    let mut evil = b.index.clone();
    evil.extensions[0].description = "Totally safe".into();
    f.http.put(&format!("{BASE}index.json"), serde_json::to_vec(&evil).unwrap());
    let got = s.fetch().unwrap();
    assert!(matches!(&got.origin, Origin::Cache { why } if why.contains("didn't check")), "{:?}", got.origin);
    assert_eq!(got.index, b.index);
    // An older index, validly signed (a replay): refused.
    f.publish("2026-01-01T00:00:00Z");
    let got = s.fetch().unwrap();
    assert!(matches!(&got.origin, Origin::Cache { why } if why.contains("older than the one it has")), "{:?}", got.origin);
    // No cache and no network: an error, never an unchecked index.
    let fresh = HttpsSource::new(f.http.clone(), f.config(), f.dir.path().join("cache2"));
    f.http.set_offline(true);
    assert!(fresh.fetch().is_err());
}

#[test]
fn an_index_signed_by_another_key_is_refused() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    f.publish(NOW);
    let mut c = f.config();
    c.keys = vec![Signing::new().signer()];
    let s = HttpsSource::new(f.http.clone(), c, f.dir.path().join("cache"));
    assert!(s.fetch().unwrap_err().contains("isn't by a key Pitwall trusts"));
    // The official index has no key yet: never read.
    assert!(IndexConfig::official().keys.is_empty());
}

#[test]
fn parsing_holds_the_limits() {
    let f = IndexFixture::new();
    f.release("ci-status", "1.0.0", PERMS);
    let b = f.build(NOW, None).unwrap();
    let ok = serde_json::to_value(&b.index).unwrap();
    let bad = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = ok.clone();
        edit(&mut v);
        parse_index(&serde_json::to_vec(&v).unwrap()).unwrap_err()
    };
    assert!(bad(&|v| v["format"] = 2.into()).contains("format 2"));
    assert!(bad(&|v| v["extensions"][0]["description"] = "x".repeat(201).into()).contains("longer than 200"));
    assert!(bad(&|v| v["extensions"][0]["name"] = "Safe\u{202E}gnp.exe".into()).contains("direction"));
    assert!(bad(&|v| v["extensions"][0]["versions"][0]["url"] = "http://github.com/x".into()).contains("release asset"));
    assert!(bad(&|v| v["extensions"][0]["versions"][0]["sha256"] = "XYZ".into()).contains("hex"));
    assert!(bad(&|v| v["extensions"][0]["screens"][0] = "../../etc/passwd".into()).contains("screenshot"));
    assert!(bad(&|v| v["extensions"][0]["keys"][0] = "nope".into()).contains("key"));
    assert!(bad(&|v| {
        let e = v["extensions"][0].clone();
        v["extensions"].as_array_mut().unwrap().push(e);
    })
    .contains("twice"));
    assert!(parse_index(&vec![b' '; (MAX_INDEX_BYTES + 1) as usize]).unwrap_err().contains("larger than"));
}

#[test]
fn the_report_link_is_prefilled() {
    let u = report_url("pitwalls/pitwall-extensions", "ci-status", "1.0.0", "9b1e", ReportKind::Malware);
    assert_eq!(u, "https://github.com/pitwalls/pitwall-extensions/issues/new?template=report.yml&title=Report%3A%20ci-status%201.0.0%20%28malware%29&labels=report&extension=ci-status&version=1.0.0&sha256=9b1e&category=malware");
}

#[test]
fn times_read_and_write() {
    assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(format_time(parse_time("2026-11-27T10:00:00Z").unwrap()), "2026-11-27T10:00:00Z");
    assert_eq!(parse_time("2026-11-27"), parse_time("2026-11-27T00:00:00Z"));
    assert_eq!(parse_time("2026-13-01"), None);
    let now = parse_time(NOW).unwrap();
    assert!(!is_stale("2026-11-20T00:00:00Z", now));
    assert!(is_stale("2026-11-01T00:00:00Z", now));
}

#[test]
fn publish_adds_a_version_or_refuses() {
    let f = IndexFixture::new();
    let v = f.release("ci-status", "1.0.0", PERMS);
    let text = std::fs::read_to_string(f.root().join("extensions/ci-status.toml")).unwrap();
    assert!(repo::add_version(&text, v.clone()).unwrap_err().contains("in the index already"));
    let older = repo::EntryVersion { version: "0.9.0".into(), ..v.clone() };
    assert!(repo::add_version(&text, older).unwrap_err().contains("isn't newer"));
    let url = repo::new_file_url("pitwalls/pitwall-extensions", "extensions/ci-status.toml", &text);
    assert!(url.starts_with("https://github.com/pitwalls/pitwall-extensions/new/main?filename=extensions%2Fci-status.toml&value=id%20%3D%20"));
}

#[test]
fn changelog_sections() {
    let log = "# Changelog\n\n## [1.1.0] - 2026-11-27\n\n- New.\n\n### Fixed\n\n- A bug.\n\n## 1.0.0\n\n- First.\n";
    assert_eq!(repo::changelog_section(log, "1.1.0").unwrap(), "- New.\n\n### Fixed\n\n- A bug.");
    assert_eq!(repo::changelog_section(log, "1.0.0").unwrap(), "- First.");
    assert_eq!(repo::changelog_section(log, "2.0.0"), None);
}

#[test]
fn categories_toml_is_pitwalls_list() {
    let c: repo::Categories = toml::from_str(&repo::categories_toml()).unwrap();
    assert_eq!(c.categories.len(), CATEGORIES.len());
}
