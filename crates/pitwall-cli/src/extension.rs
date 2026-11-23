//! `pitwall extension check|test|new` (docs/extensions/checks.md): for an
//! extension's author, on a folder, offline: no socket, no running app.
//! The work is `pitwall_core::extensions::{check, starter}`; Pitwall's own
//! tests run the same checks on the extensions in its repository.

use pitwall_client::Error;
use pitwall_core::extensions::{check, starter};
use pitwall_proto::{code, ErrorBody};
use serde_json::json;

use crate::args::ExtensionCmd;
use crate::Output;

/// Run `cmd`: its output and whether every check passed.
pub fn run(cmd: &ExtensionCmd) -> Result<(Output, bool), Error> {
    Ok(match cmd {
        ExtensionCmd::Check { folder } => report(check::check(folder, None)),
        ExtensionCmd::Test { folder } => report(check::test(folder, None)),
        ExtensionCmd::New { folder, name, protocol, view } => {
            let files = starter::write(folder, name.as_deref(), *protocol, *view).map_err(|e| Error::Server(ErrorBody::new(code::BAD_PARAMS, e)))?;
            let m = pitwall_core::extensions::read_in_place(folder).map_err(|e| Error::Server(ErrorBody::new("failed", e)))?;
            let shown = folder.display();
            let mut human = format!("Made {} ({}) in {shown}:\n", m.name, m.id);
            for f in &files {
                human.push_str(&format!("  {}\n", f.strip_prefix(folder).unwrap_or(f).display()));
            }
            human.push_str(&format!("\nNext:\n  pitwall extension check {shown}\n  pitwall extension test {shown}\n  In Pitwall: Settings → Extensions → Load from folder…, then turn it on.\n"));
            let files: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
            (Output { json: json!({ "folder": folder, "id": m.id, "name": m.name, "files": files }), human }, true)
        }
    })
}

fn report(r: check::Report) -> (Output, bool) {
    let passed = r.passed();
    let mut json = serde_json::to_value(&r).unwrap_or_default();
    json["passed"] = json!(passed);
    (Output { json, human: r.text() }, passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The starter, made and checked as an author would.
    #[test]
    fn new_then_check_and_test() {
        let t = pitwall_core::testing::TempDir::new("cli-ext");
        let folder = t.path().join("hello-ext");
        let (o, ok) = run(&ExtensionCmd::New { folder: folder.clone(), name: Some("Hello".into()), protocol: 1, view: false }).unwrap();
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
        let Err(Error::Server(e)) = run(&ExtensionCmd::New { folder, name: None, protocol: 1, view: false }) else { panic!("made twice") };
        assert_eq!(e.code, code::BAD_PARAMS);
        // A folder that isn't an extension fails, with exit status 1.
        let (o, ok) = run(&ExtensionCmd::Check { folder: t.path().join("nope") }).unwrap();
        assert!(!ok && o.human.contains("FAIL"));
    }
}
