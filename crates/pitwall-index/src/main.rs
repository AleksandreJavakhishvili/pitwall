//! `pitwall-index`: the extensions index repository's tool
//! (docs/spec/tile-sdk/marketplace.md §7.2). See the crate's README.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use pitwall_core::extensions::index;

#[derive(Parser)]
#[command(name = "pitwall-index", about = "Check and build the Pitwall extensions index.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check the index repository (a pull request's checkout): every file,
    /// the change against the base branch, and each new version
    /// downloaded, checked and run headless. Prints a Markdown report;
    /// exits 1 when a check fails. Writes `auto-merge=true|false` to
    /// --output (a GitHub Actions output file) when given.
    Check {
        /// The checkout to check.
        repo: PathBuf,
        /// The base branch's checkout, to judge the change against.
        #[arg(long)]
        base: Option<PathBuf>,
        /// The pull request author's GitHub login.
        #[arg(long, default_value = "")]
        login: String,
        /// The index built last (the base's dist/v1/index.json): versions
        /// in it aren't downloaded again.
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Skip the headless run (only for a machine without the tile
        /// runtime).
        #[arg(long)]
        no_run: bool,
        /// Append `auto-merge=…` to this file.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Build dist/v1/ from the repository and sign index.json and
    /// blocked.json with the index key (its password, if any, from
    /// PITWALL_INDEX_KEY_PASSWORD).
    Build {
        repo: PathBuf,
        /// Where to write (default: <repo>/dist/v1).
        #[arg(long)]
        out: Option<PathBuf>,
        /// The index's minisign secret key.
        #[arg(long)]
        key: PathBuf,
        /// Skip the headless run.
        #[arg(long)]
        no_run: bool,
    },
    /// Record a maintainer's review of one version (Verified): signs the
    /// review statement with the maintainer's review key (its password
    /// from PITWALL_REVIEW_KEY_PASSWORD) and writes it into the entry.
    Review {
        repo: PathBuf,
        id: String,
        version: String,
        /// The maintainer's handle in maintainers.toml.
        #[arg(long)]
        by: String,
        /// The maintainer's review secret key.
        #[arg(long)]
        key: PathBuf,
        /// The component rebuilt from the entry's commit gave the same
        /// hash. Required: only a reproducible version is Verified.
        #[arg(long)]
        reproducible: bool,
    },
    /// Print categories.toml (Pitwall's fixed list).
    Categories,
}

fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    index::format_time(secs)
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Check { repo, base, login, previous, no_run, output } => {
            let previous = previous.and_then(|p| std::fs::read(p).ok()).and_then(|b| index::parse_index(&b).ok());
            let r = pitwall_index::check(&repo, base.as_deref(), &login, previous.as_ref(), &pitwall_index::Https, !no_run, &now());
            print!("{}", r.markdown());
            if let Some(o) = output {
                use std::io::Write;
                let line = format!("auto-merge={}\n", r.auto_merge());
                if let Err(e) = std::fs::OpenOptions::new().create(true).append(true).open(&o).and_then(|mut f| f.write_all(line.as_bytes())) {
                    eprintln!("{}: {e}", o.display());
                }
            }
            if r.passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Cmd::Build { repo, out, key, no_run } => {
            let out = out.unwrap_or_else(|| repo.join("dist/v1"));
            let key = match pitwall_index::secret_key(&key, "PITWALL_INDEX_KEY_PASSWORD") {
                Ok(k) => k,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            };
            match pitwall_index::build(&repo, &out, &key, &pitwall_index::Https, &now(), !no_run) {
                Ok(w) => {
                    println!("Built {} ({} extensions, {} new versions, {} new screenshots).", out.display(), w.extensions, w.new_versions.len(), w.screens);
                    ExitCode::SUCCESS
                }
                Err(p) => {
                    for line in p {
                        eprintln!("- {line}");
                    }
                    ExitCode::FAILURE
                }
            }
        }
        Cmd::Review { repo, id, version, by, key, reproducible } => {
            let done = pitwall_index::secret_key(&key, "PITWALL_REVIEW_KEY_PASSWORD").and_then(|k| pitwall_index::review(&repo, &id, &version, &by, &k, &now()[..10], reproducible));
            match done {
                Ok(()) => {
                    println!("{id} {version} is Verified by {by}: commit extensions/{id}.toml in a pull request.");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Cmd::Categories => {
            print!("{}", index::repo::categories_toml());
            ExitCode::SUCCESS
        }
    }
}
