//! Extensions from an npm package, against a registry standing in on this
//! machine's loopback address (`testing::NpmRegistry`): no request leaves
//! this machine.

use super::*;
use crate::testing::npm::{gzip, tar, tgz, LoopbackHttp, NpmRegistry, TarItem};
use crate::testing::{npm_extension, TempDir};

const REGISTRY: &str = "https://registry.example.com";
const PKG: &str = "@me/inbox";

fn rel(version: &str, deprecated: bool) -> Release {
    Release { version: version.into(), integrity: Some(integrity_of(version.as_bytes())), tarball: format!("{REGISTRY}/x.tgz"), deprecated }
}

/// The names in folder `dir`.
fn names(dir: &Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    n.sort();
    n
}

#[test]
fn names_addresses_and_integrity() {
    assert_eq!(metadata_url("https://r.example.com/", "@me/x"), "https://r.example.com/@me%2fx");
    assert_eq!(metadata_url(REGISTRY, "x"), "https://registry.example.com/x");
    let via = vec![(REGISTRY.to_string(), "http://127.0.0.1:9".to_string())];
    assert_eq!(routed("https://registry.example.com/x/-/x-1.0.0.tgz", &via), "http://127.0.0.1:9/x/-/x-1.0.0.tgz");
    assert_eq!(routed("https://other.example.com/x", &via), "https://other.example.com/x");
    let i = integrity_of(b"hello");
    assert!(i.starts_with("sha512-") && i.len() == 7 + 88, "{i}");
    assert_eq!(sha512_of(&format!("sha1-AAAA {i}")).as_deref(), Some(i.as_str()));
    assert_eq!(sha512_of("sha1-2jmj7l5rSw0yVb/vlWAYkK/YBwk="), None, "sha1 alone isn't enough");
    assert_eq!(sha512_of("sha512-short"), None);
    assert_eq!(sha512_of("sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="), None);
    assert!(short(&i).starts_with("sha512 ") && short(&i).ends_with('…'));
}

/// Versions newest first; the newest stable one offered, a pre-release
/// only to an installed pre-release, a deprecated one only when every
/// candidate is.
#[test]
fn the_newest_version_skips_pre_releases_and_deprecated_ones() {
    let doc = serde_json::json!({
        "name": PKG,
        "versions": {
            "1.0.0": {"version": "1.0.0", "dist": {"tarball": "https://r/x-1.0.0.tgz", "integrity": integrity_of(b"1")}},
            "1.10.0": {"version": "1.10.0", "dist": {"tarball": "https://r/x-1.10.0.tgz", "integrity": integrity_of(b"2")}, "deprecated": "broken"},
            "1.9.0": {"version": "1.9.0", "dist": {"tarball": "https://r/x-1.9.0.tgz", "integrity": integrity_of(b"3")}},
            "2.0.0-beta.1": {"version": "2.0.0-beta.1", "dist": {"tarball": "https://r/x-2.tgz", "integrity": integrity_of(b"4")}},
            "0.9.0": {"version": "0.9.0", "dist": {"tarball": "https://r/x-0.9.0.tgz", "shasum": "abc"}},
            "latest": {"version": "latest", "dist": {"tarball": "https://r/y.tgz"}},
            "1.11.0": {"version": "1.11.0"}
        }
    });
    let list = parse_releases(&serde_json::to_vec(&doc).unwrap(), PKG).unwrap();
    let vs: Vec<&str> = list.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(vs, ["2.0.0-beta.1", "1.10.0", "1.9.0", "1.0.0", "0.9.0"], "not a version, or no tarball: left out");
    assert!(list[1].deprecated && list[4].integrity.is_none());
    assert_eq!(newest(&list, false).map(|r| r.version.as_str()), Some("1.9.0"), "deprecated 1.10.0 skipped, no pre-release");
    assert_eq!(newest(&list, true).map(|r| r.version.as_str()), Some("2.0.0-beta.1"));
    assert_eq!(newest(&[rel("1.1.0", true), rel("1.0.0", true)], false).map(|r| r.version.as_str()), Some("1.1.0"), "all deprecated: the newest");
    assert_eq!(newest(&[rel("1.1.0-rc.1", false)], false), None);
    assert!(newest(&[], false).is_none());
    assert!(parse_releases(br#"{"name": "@me/other", "versions": {}}"#, PKG).unwrap_err().contains("another package"));
    assert!(parse_releases(b"<html>", PKG).unwrap_err().contains("isn't package metadata"));
}

/// The equivalent of a moved tag: a version whose integrity isn't the one
/// installed or seen before is refused, as is one without sha512.
#[test]
fn a_changed_version_is_refused() {
    let r = rel("1.1.0", false);
    let i = r.integrity.clone().unwrap();
    assert_eq!(refusal(PKG, &r, None, None), None);
    assert_eq!(refusal(PKG, &r, Some(("1.1.0", &i)), Some(&i)), None);
    assert_eq!(refusal(PKG, &r, Some(("1.0.0", "sha512-other")), None), None, "another version says nothing");
    let other = integrity_of(b"other");
    assert!(refusal(PKG, &r, Some(("1.1.0", &other)), None).unwrap().starts_with("Version 1.1.0 of @me/inbox changed on the registry: its integrity was sha512 "));
    assert!(refusal(PKG, &r, None, Some(&other)).is_some(), "against an earlier check");
    let bare = Release { integrity: None, ..r };
    assert!(refusal(PKG, &bare, None, None).unwrap().contains("has no sha512 integrity"));
}

/// Listing and fetching from the stand-in registry: the version's files
/// unpacked into a hidden folder (an executable kept executable), no
/// credentials sent, and nothing else written.
#[test]
fn a_version_is_listed_and_fetched() {
    let reg = NpmRegistry::start(REGISTRY);
    let i1 = reg.publish(PKG, "1.0.0", &npm_extension(PKG, REGISTRY, "inbox", "1.0.0", "// 1", "[]"));
    reg.publish(PKG, "1.1.0", &npm_extension(PKG, REGISTRY, "inbox", "1.1.0", "// 2", "[]"));
    let list = releases(REGISTRY, PKG, &reg.via(), &LoopbackHttp).unwrap();
    assert_eq!(list.iter().map(|r| r.version.as_str()).collect::<Vec<_>>(), ["1.1.0", "1.0.0"]);
    assert_eq!(list[1].integrity.as_deref(), Some(i1.as_str()));
    let t = TempDir::new("npm-fetch");
    let work = t.path().join("data/extensions");
    let f = fetch(REGISTRY, PKG, "1.1.0", list[0].integrity.as_deref(), Some("inbox"), &work, &reg.via(), &LoopbackHttp).unwrap();
    assert_eq!(f.manifest.version.as_deref(), Some("1.1.0"));
    assert_eq!(std::fs::read_to_string(f.folder.join("adapter/main.mjs")).unwrap(), "// 2");
    assert_eq!(names(&f.folder), ["adapter", "bin", "extension.json", "package.json"]);
    if let Some(mode) = crate::platform::mode(&f.folder.join("bin/run")) {
        assert_eq!(mode & 0o111, 0o111, "executable, as packed");
        assert_eq!(crate::platform::mode(&f.folder.join("adapter/main.mjs")).unwrap() & 0o111, 0);
    }
    let left = names(&work);
    assert_eq!(left.len(), 1, "only the fetched folder: {left:?}");
    assert!(left[0].starts_with(".fetched-"));
    for head in reg.requests() {
        let low = head.to_lowercase();
        assert!(!low.contains("authorization") && !low.contains("cookie"), "no credentials: {head}");
    }
    assert!(reg.requests()[0].starts_with("GET /@me%2finbox ") && reg.requests()[0].contains("application/vnd.npm.install-v1+json"), "{:?}", reg.requests());
    // The older one, with any id.
    std::fs::remove_dir_all(&f.folder).unwrap();
    let f = fetch(REGISTRY, PKG, "1.0.0", None, None, &work, &reg.via(), &LoopbackHttp).unwrap();
    assert_eq!(std::fs::read_to_string(f.folder.join("adapter/main.mjs")).unwrap(), "// 1");
}

/// What doesn't match the registry's integrity, the version offered, or
/// the caller's extension is refused, and leaves nothing.
#[test]
fn a_fetch_refuses_what_doesnt_match() {
    let reg = NpmRegistry::start(REGISTRY);
    let t = TempDir::new("npm-refuse");
    let work = t.path().join("work");
    let get = |v: &str, i: Option<&str>, id: Option<&str>| fetch(REGISTRY, PKG, v, i, id, &work, &reg.via(), &LoopbackHttp).map(|f| f.folder);
    // A download that doesn't match the integrity the metadata gives.
    reg.publish_with(PKG, "1.0.0", &npm_extension(PKG, REGISTRY, "inbox", "1.0.0", "// 1", "[]"), Some(&integrity_of(b"something else")));
    assert!(get("1.0.0", None, None).unwrap_err().contains("doesn't match the integrity the registry gives"));
    // Integrity changed since it was offered (republished).
    let i = reg.publish(PKG, "1.0.0", &npm_extension(PKG, REGISTRY, "inbox", "1.0.0", "// 1", "[]"));
    let was = integrity_of(b"what was offered");
    assert!(get("1.0.0", Some(&was), None).unwrap_err().contains("Version 1.0.0 of @me/inbox changed on the registry"));
    assert!(get("1.0.0", Some(&i), Some("other")).unwrap_err().contains("holds extension “inbox”, not “other”"));
    // A manifest of another version than the package's.
    reg.publish(PKG, "1.0.1", &npm_extension(PKG, REGISTRY, "inbox", "1.0.0", "// 1", "[]"));
    assert!(get("1.0.1", None, None).unwrap_err().contains("@me/inbox 1.0.1's extension.json says version 1.0.0, not 1.0.1"));
    // No manifest at the package's root.
    reg.publish(PKG, "1.0.2", &tgz(&[TarItem::file("package/package.json", "{}"), TarItem::file("package/ext/extension.json", "{}")]));
    assert!(get("1.0.2", None, None).unwrap_err().contains("has no extension.json in the package's root"));
    // Only a sha1 shasum.
    reg.publish_with(PKG, "1.0.3", &npm_extension(PKG, REGISTRY, "inbox", "1.0.3", "// 3", "[]"), None);
    assert!(get("1.0.3", None, None).unwrap_err().contains("has no sha512 integrity"));
    assert!(get("9.9.9", None, None).unwrap_err().contains("isn't on the registry"));
    assert!(names(&work).is_empty(), "nothing left behind: {:?}", names(&work));
}

/// A bad package name or a registry that isn't https is refused before
/// anything is asked; a package the registry doesn't have, or a registry
/// that can't be reached, is said in plain words.
#[test]
fn bad_names_registries_and_offline_are_refused() {
    let reg = NpmRegistry::start(REGISTRY);
    assert!(releases(REGISTRY, "../etc/passwd", &reg.via(), &LoopbackHttp).unwrap_err().contains("isn't an npm package name"));
    assert!(releases(REGISTRY, "x@1.0.0", &reg.via(), &LoopbackHttp).unwrap_err().contains("isn't an npm package name"));
    assert!(releases("http://registry.example.com", PKG, &reg.via(), &LoopbackHttp).unwrap_err().contains("https://"));
    assert!(releases("https://me:pw@registry.example.com", PKG, &reg.via(), &LoopbackHttp).unwrap_err().contains("user or password"));
    assert!(reg.requests().is_empty(), "nothing asked");
    assert!(releases(REGISTRY, "@me/nothing", &reg.via(), &LoopbackHttp).unwrap_err().contains("has no such package"));
    // A port nothing listens on (taken and let go).
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let gone = vec![(REGISTRY.to_string(), format!("http://127.0.0.1:{port}"))];
    assert!(releases(REGISTRY, PKG, &gone, &LoopbackHttp).unwrap_err().contains("Couldn't reach it"));
}

/// Metadata over 5 MiB, a tarball over 50 MiB, and a tarball holding more
/// than 50 MiB or 5000 files unpacked: refused.
#[test]
fn the_limits_hold() {
    let reg = NpmRegistry::start(REGISTRY);
    let t = TempDir::new("npm-limits");
    let work = t.path().join("work");
    reg.serve_raw("/@me%2fbig", vec![b' '; MAX_METADATA as usize + 1]);
    assert!(releases(REGISTRY, "@me/big", &reg.via(), &LoopbackHttp).unwrap_err().contains("metadata of @me/big is bigger than 5 MiB"));
    // Random bytes don't compress: a download over the limit.
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let big: Vec<u8> = (0..MAX_BYTES + 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    reg.publish(PKG, "1.0.0", &big);
    assert_eq!(fetch(REGISTRY, PKG, "1.0.0", None, None, &work, &reg.via(), &LoopbackHttp).unwrap_err(), "It is bigger than 50 MiB. Not installed.");
    drop(big);
    // Small compressed, too big unpacked.
    let zeros = tgz(&[TarItem::file("package/extension.json", "{}"), TarItem::file("package/zeros", vec![0u8; MAX_BYTES as usize + 1])]);
    assert!(zeros.len() < 1024 * 1024);
    reg.publish(PKG, "1.0.1", &zeros);
    assert!(fetch(REGISTRY, PKG, "1.0.1", None, None, &work, &reg.via(), &LoopbackHttp).unwrap_err().contains("bigger than 50 MiB unpacked"));
    let many: Vec<TarItem> = (0..=MAX_FILES).map(|i| TarItem::file(&format!("package/f{i}"), "x")).collect();
    reg.publish(PKG, "1.0.2", &tgz(&many));
    assert!(fetch(REGISTRY, PKG, "1.0.2", None, None, &work, &reg.via(), &LoopbackHttp).unwrap_err().contains("more than 5000 files"));
    assert!(names(&work).is_empty(), "nothing left behind");
}

/// Unpacking refuses links, devices, pipes, absolute paths, `..` and
/// anything outside `package/`, whatever comes before; nothing is ever
/// written outside the folder it unpacks into.
#[test]
fn unpacking_refuses_what_isnt_plain_files() {
    let t = TempDir::new("npm-unpack");
    let outside = t.path().join("evil.txt");
    let abs = t.path().join("abs.txt").to_string_lossy().replace('\\', "/");
    let cases: Vec<(TarItem, &str)> = vec![
        (TarItem::link(b'2', "package/link", "../../evil.txt"), "a symbolic link (package/link)"),
        (TarItem::link(b'2', "package/inside", "adapter/main.mjs"), "a symbolic link"),
        (TarItem::link(b'1', "package/hard", "package/extension.json"), "a hard link (package/hard)"),
        (TarItem::special(b'3', "package/tty"), "a device"),
        (TarItem::special(b'4', "package/disk"), "a device"),
        (TarItem::special(b'6', "package/fifo"), "a pipe"),
        (TarItem::special(b'S', "package/sparse"), "a kind Pitwall doesn't unpack"),
        (TarItem::file("package/../../evil.txt", "x"), "leaves its folder"),
        (TarItem::file("../evil.txt", "x"), "leaves its folder"),
        (TarItem::file(&abs, "x"), "an absolute path"),
        (TarItem::file("evil.txt", "x"), "outside package/"),
        (TarItem::file("other/evil.txt", "x"), "outside package/"),
        (TarItem::file("package/a\\..\\..\\evil.txt", "x"), "doesn't unpack"),
        (TarItem::file("package/extension.json", "again"), "twice"),
        (TarItem::file("package", "x"), "where its folder should be"),
    ];
    for (i, (bad, why)) in cases.into_iter().enumerate() {
        let dest = t.path().join(format!("d{i}"));
        let items = [TarItem::file("package/extension.json", "{}"), TarItem::dir("package/adapter/"), TarItem::file("package/adapter/main.mjs", "//"), bad.clone()];
        let e = unpack(&tgz(&items), &dest).unwrap_err();
        assert!(e.contains(why) && e.ends_with("Not installed."), "{}: {e}", bad.name);
        for entry in std::fs::read_dir(t.path()).unwrap().flatten() {
            let n = entry.file_name().to_string_lossy().into_owned();
            assert!(n.starts_with('d'), "{}: wrote {n} outside", bad.name);
        }
        assert!(!outside.exists() && !Path::new(&abs).exists());
        // What it wrote before the refusal is in its folder only, and no
        // link was made.
        let mut stack = vec![dest.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let ft = std::fs::symlink_metadata(e.path()).unwrap().file_type();
                assert!(!ft.is_symlink(), "{}: a link", bad.name);
                if ft.is_dir() {
                    stack.push(e.path());
                }
            }
        }
    }
    // Not a tarball, a cut one, a damaged header.
    assert!(unpack(b"this is not gzip, nor a tarball at all", &t.path().join("e1")).unwrap_err().contains("isn't a package Pitwall can unpack"));
    let whole = tar(&[TarItem::file("package/extension.json", "{}")]);
    assert!(unpack(&gzip(&whole[..600]), &t.path().join("e2")).unwrap_err().contains("isn't a complete package"));
    let mut damaged = whole.clone();
    damaged[0] = b'q';
    assert!(unpack(&gzip(&damaged), &t.path().join("e3")).unwrap_err().contains("a damaged header"));
}

/// What a real package holds unpacks: long names (GNU and pax), folders
/// given or not, `./` parts; node_modules and a marker of Pitwall's are
/// left out.
#[test]
fn unpacking_keeps_plain_files() {
    let t = TempDir::new("npm-unpack-ok");
    let long = format!("package/{}/main.mjs", "d".repeat(120));
    let pax_name = format!("package/{}.txt", "p".repeat(150));
    let record = format!("path={pax_name}\n");
    let len = (record.len() + 4).to_string().len() + 1 + record.len();
    let pax = TarItem { name: "PaxHeader".into(), kind: b'x', data: format!("{len} {record}").into_bytes(), mode: 0o644, link: String::new() };
    let items = [
        TarItem::file("package/extension.json", "{}"),
        TarItem::file("./package/./lib/a.js", "a"),
        TarItem::file(&long, "long"),
        pax,
        TarItem::file("package/short-name-replaced", "pax"),
        TarItem::file("package/node_modules/dep/index.js", "no"),
        TarItem::file(&format!("package/{}", crate::extensions::install::MARKER), "no"),
    ];
    let dest = t.path().join("out");
    unpack(&tgz(&items), &dest).unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("lib/a.js")).unwrap(), "a");
    assert_eq!(std::fs::read_to_string(dest.join(long.trim_start_matches("package/"))).unwrap(), "long");
    assert_eq!(std::fs::read_to_string(dest.join(pax_name.trim_start_matches("package/"))).unwrap(), "pax");
    assert!(!dest.join("short-name-replaced").exists() && !dest.join("node_modules").exists() && !dest.join(crate::extensions::install::MARKER).exists());
}
