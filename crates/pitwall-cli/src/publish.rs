//! `pitwall extension publish` (docs/spec/tile-sdk/marketplace.md §7.2):
//! a signed package's entry for the extensions index, and the link that
//! opens its pull request. Offline: it writes files and prints links;
//! the author releases, commits, pushes and opens the pull request.

use std::path::{Path, PathBuf};

use pitwall_client::Error;
use pitwall_core::extensions::index::{self, repo};
use pitwall_core::extensions::package;
use pitwall_proto::{code, ErrorBody};
use serde_json::json;

use crate::args::ExtensionCmd;
use crate::Output;

fn bad(e: impl Into<String>) -> Error {
    Error::Server(ErrorBody::new(code::BAD_PARAMS, e.into()))
}

/// The commit of folder `dir`'s git checkout, when it is one.
fn head_commit(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(["rev-parse", "HEAD"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|c| index::is_commit(c))
}

fn write(path: &Path, text: &str) -> Result<(), Error> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| bad(format!("Can't make {}: {e}", p.display())))?;
    }
    std::fs::write(path, text).map_err(|e| bad(format!("Can't write {}: {e}", path.display())))
}

/// Run `pitwall extension publish` (`cmd` is `ExtensionCmd::Publish`).
pub fn publish(cmd: &ExtensionCmd) -> Result<Output, Error> {
    let ExtensionCmd::Publish { file, repository, author, github, categories, license, commit, url, index_dir, update, index: index_repo } = cmd else { unreachable!() };
    let repository = repository.trim_end_matches('/').to_string();
    index::check_repository(&repository).map_err(|e| bad(format!("--repository: {e}.")))?;
    pitwall_core::extensions::check_id(author).map_err(|_| bad("--author is your handle in the index: lower-case letters, digits and -, starting with a letter."))?;
    let github = github.clone().unwrap_or_else(|| repository.trim_start_matches("https://github.com/").split('/').next().unwrap_or_default().to_string());

    // The package, as Pitwall will check it: signed, its rules.
    let opened = package::open(file, None).map_err(|why| bad(format!("{} can't be published: {why}", file.display())))?;
    let signer = opened.signer.clone().ok_or_else(|| bad(format!("{} isn't signed: sign it with `pitwall extension sign {}` and release the .minisig beside it.", file.display(), file.display())))?;
    let m = &opened.manifest;
    let version = m.version.clone().unwrap_or_default();
    if !opened.files.iter().any(|f| f.name.starts_with("LICENSE")) {
        return Err(bad("The package has no LICENSE: the index needs one. Add it to the folder and pack again."));
    }
    if m.description.as_deref().is_none_or(|d| d.trim().is_empty()) {
        return Err(bad("Its extension.json has no description: the index shows one. Add it and pack again."));
    }
    let world = m.contributes.tiles.first().and_then(|t| t.world.clone()).unwrap_or_else(|| repo::DEFAULT_WORLD.to_string());
    let folder = file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let commit = match commit.clone().or_else(|| head_commit(folder)) {
        Some(c) if index::is_commit(&c) => c,
        Some(c) => return Err(bad(format!("--commit {c} isn't a full commit id (40 hex digits)."))),
        None => return Err(bad("Name the commit the component was built from with --commit (the folder isn't a git checkout).")),
    };
    let file_name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let url = url.clone().unwrap_or_else(|| format!("{repository}/releases/download/v{version}/{file_name}"));
    index::check_release_url(&url, &repository).map_err(|e| bad(format!("{e}.")))?;
    let v = repo::EntryVersion { version: version.clone(), url: url.clone(), sha256: opened.sha256.clone(), commit, pitwall: m.pitwall.clone(), world, yanked: false, review: None };

    let entry_path = format!("extensions/{}.toml", m.id);
    let author_path = format!("authors/{author}.toml");
    let new_author = repo::Author { handle: author.clone(), github: github.clone(), keys: vec![signer.public_key.clone()], contact: None };
    let new_entry = || -> Result<repo::Entry, Error> {
        if categories.is_empty() || categories.len() > index::MAX_CATEGORIES {
            return Err(bad(format!("A new extension needs 1 to {} --category ({}).", index::MAX_CATEGORIES, index::CATEGORIES.map(|c| c.0).join(", "))));
        }
        if let Some(c) = categories.iter().find(|c| index::category_name(c).is_none()) {
            return Err(bad(format!("“{c}” isn't a category: {}.", index::CATEGORIES.map(|c| c.0).join(", "))));
        }
        let license = license.clone().ok_or_else(|| bad("A new extension needs --license (SPDX: MIT, Apache-2.0…)."))?;
        Ok(repo::new_entry(m, author, &repository, categories.clone(), &license, v.clone()))
    };

    let mut human = format!("{} {} (sha256 {}, signed by key {}):\n", m.name, version, opened.sha256, signer.key_id);
    let mut json = json!({ "id": m.id, "version": version, "sha256": opened.sha256, "url": url, "keyId": signer.key_id, "publicKey": signer.public_key });
    let release = format!("Before the pull request, release both files on GitHub at:\n  {url}\n  {url}.minisig\n");

    if let Some(dir) = index_dir {
        // A clone of the index: read and write there.
        let existing = std::fs::read_to_string(dir.join(&entry_path)).ok();
        let e = match &existing {
            Some(text) => {
                let e = repo::add_version(text, v.clone()).map_err(bad)?;
                if e.author != *author {
                    return Err(bad(format!("{} belongs to {} in the index, not {author}.", m.id, e.author)));
                }
                e
            }
            None => new_entry()?,
        };
        let mut written = vec![entry_path.clone()];
        write(&dir.join(&entry_path), &repo::entry_toml(&e))?;
        match std::fs::read_to_string(dir.join(&author_path)).ok().and_then(|t| repo::parse_author(&t).ok()) {
            None => {
                write(&dir.join(&author_path), &repo::author_toml(&new_author))?;
                written.push(author_path.clone());
            }
            Some(mut a) if !a.keys.contains(&signer.public_key) => {
                a.keys.push(signer.public_key.clone());
                write(&dir.join(&author_path), &repo::author_toml(&a))?;
                written.push(author_path.clone());
                human.push_str("  Your key is new to the index: it is added to your author file, and a maintainer checks key changes.\n");
            }
            Some(_) => {}
        }
        // The same checks the index's CI runs first.
        if let Err(problems) = repo::load(dir) {
            human.push_str("\nThe index's checks would fail:\n");
            for p in &problems {
                human.push_str(&format!("  - {p}\n"));
            }
            json["problems"] = json!(problems);
        }
        let branch = format!("add-{}-{version}", m.id);
        let title = if existing.is_some() { format!("{} {version}", m.id) } else { format!("Add {}", m.id) };
        let pr = format!("https://github.com/{index_repo}/compare/main...{github}:{branch}?{}", index::query(&[("quick_pull", "1"), ("title", &title)]));
        human.push_str(&format!("Wrote in {}:\n", dir.display()));
        for w in &written {
            human.push_str(&format!("  {w}\n"));
        }
        human.push_str(&format!("\n{release}\nThen, in {}:\n  git switch -c {branch}\n  git add {}\n  git commit -m \"{title}\"\n  git push -u origin {branch}\nand open the pull request:\n  {pr}\n", dir.display(), written.join(" ")));
        json["written"] = json!(written.iter().map(|w| dir.join(w)).collect::<Vec<PathBuf>>());
        json["pullRequest"] = json!(pr);
        return Ok(Output { json, human, failed: None });
    }

    let out_dir = folder.join("index-entry");
    if *update {
        let block = toml_versions_block(&v);
        let edit = repo::edit_url(index_repo, &entry_path);
        human.push_str(&format!("\n{release}\nThen add this to {entry_path} (opens GitHub's editor, which makes the pull request):\n  {edit}\n\n{block}"));
        json["versions"] = json!(block);
        json["edit"] = json!(edit);
        return Ok(Output { json, human, failed: None });
    }
    let e = new_entry()?;
    let entry = repo::entry_toml(&e);
    let author_text = repo::author_toml(&new_author);
    write(&out_dir.join(&entry_path), &entry)?;
    write(&out_dir.join(&author_path), &author_text)?;
    let entry_link = repo::new_file_url(index_repo, &entry_path, &entry);
    let author_link = repo::new_file_url(index_repo, &author_path, &author_text);
    human.push_str(&format!(
        "Wrote {}/{entry_path} and {author_path}.\n\n{release}\nThen open the pull request (GitHub's new-file page, prefilled; saving makes a fork and the pull request):\n  {entry_link}\nFirst time in the index? Add your author file to the same pull request (on its branch, Add file → Create new file, or this link on your fork):\n  {author_link}\n",
        out_dir.display()
    ));
    json["written"] = json!([out_dir.join(&entry_path), out_dir.join(&author_path)]);
    json["pullRequest"] = json!(entry_link);
    json["authorFile"] = json!(author_link);
    Ok(Output { json, human, failed: None })
}

/// `[[versions]]` with `v`, as an entry file holds it.
fn toml_versions_block(v: &repo::EntryVersion) -> String {
    let e = repo::Entry { id: String::new(), author: String::new(), repository: String::new(), categories: Vec::new(), license: String::new(), delisted: None, versions: vec![v.clone()] };
    let text = repo::entry_toml(&e);
    text.find("[[versions]]").map(|i| text[i..].to_string()).unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::testing::index::{tile_package, IndexFixture, COMPONENT};
    use pitwall_core::testing::{Signing, TempDir};

    fn signed(t: &TempDir, s: &Signing, version: &str) -> PathBuf {
        let bytes = tile_package("ci-status", version, "", &COMPONENT);
        let file = t.path().join(format!("ci-status-{version}.pwtile"));
        std::fs::write(&file, &bytes).unwrap();
        std::fs::write(package::signature_path(&file), s.sign(&bytes)).unwrap();
        file
    }

    fn cmd(file: PathBuf, index_dir: Option<PathBuf>, update: bool) -> ExtensionCmd {
        ExtensionCmd::Publish {
            file,
            repository: "https://github.com/octo/pitwall-ci-status".into(),
            author: "octo".into(),
            github: None,
            categories: vec!["ci".into()],
            license: Some("MIT".into()),
            commit: Some("4f1c".repeat(10)),
            url: None,
            index_dir,
            update,
            index: index::OFFICIAL_REPO.into(),
        }
    }

    fn err(r: Result<Output, Error>) -> String {
        let Err(e) = r else { panic!("it should fail") };
        match e {
            Error::Server(b) => b.message,
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn a_first_submission_writes_the_entry_and_a_prefilled_link() {
        let t = TempDir::new("publish");
        let s = Signing::new();
        let out = publish(&cmd(signed(&t, &s, "1.0.0"), None, false)).unwrap();
        let entry = std::fs::read_to_string(t.path().join("index-entry/extensions/ci-status.toml")).unwrap();
        assert!(entry.contains("url = \"https://github.com/octo/pitwall-ci-status/releases/download/v1.0.0/ci-status-1.0.0.pwtile\""), "{entry}");
        let author = std::fs::read_to_string(t.path().join("index-entry/authors/octo.toml")).unwrap();
        assert!(author.contains(&s.signer().public_key));
        let link = out.json["pullRequest"].as_str().unwrap();
        assert!(link.starts_with("https://github.com/pitwalls/pitwall-extensions/new/main?filename=extensions%2Fci-status.toml&value="), "{link}");
        assert!(out.human.contains("release both files on GitHub"));
    }

    #[test]
    fn an_unsigned_package_isnt_published() {
        let t = TempDir::new("publish");
        let file = signed(&t, &Signing::new(), "1.0.0");
        std::fs::remove_file(package::signature_path(&file)).unwrap();
        assert!(err(publish(&cmd(file, None, false))).contains("isn't signed"));
    }

    #[test]
    fn into_a_clone_of_the_index_an_update_adds_a_version_that_its_checks_accept() {
        let f = IndexFixture::new();
        f.release("ci-status", "1.0.0", "");
        let t = TempDir::new("publish");
        let out = publish(&cmd(signed(&t, &f.author, "1.1.0"), Some(f.root()), false)).unwrap();
        assert!(out.json.get("problems").is_none(), "{}", out.human);
        let e = repo::parse_entry(&std::fs::read_to_string(f.root().join("extensions/ci-status.toml")).unwrap()).unwrap();
        assert_eq!(e.versions.len(), 2);
        assert!(out.json["pullRequest"].as_str().unwrap().contains("/compare/main...octo:add-ci-status-1.1.0?quick_pull=1"));
        // The same version again: refused.
        assert!(err(publish(&cmd(signed(&t, &f.author, "1.1.0"), Some(f.root()), false))).contains("in the index already"));
        // A new key is added to the author's file, for a maintainer to check.
        let out = publish(&cmd(signed(&t, &Signing::new(), "1.2.0"), Some(f.root()), false)).unwrap();
        assert!(out.human.contains("Your key is new to the index"));
    }

    #[test]
    fn an_update_without_a_clone_prints_the_versions_block() {
        let t = TempDir::new("publish");
        let out = publish(&cmd(signed(&t, &Signing::new(), "1.1.0"), None, true)).unwrap();
        let block = out.json["versions"].as_str().unwrap();
        assert!(block.starts_with("[[versions]]\nversion = \"1.1.0\""), "{block}");
        assert_eq!(out.json["edit"], "https://github.com/pitwalls/pitwall-extensions/edit/main/extensions/ci-status.toml");
    }
}
