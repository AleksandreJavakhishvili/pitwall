# pitwall-index

The extensions index repository's tool (docs/spec/tile-sdk/marketplace.md
§7). Its workflows install it from this repository at a pinned commit;
it isn't shipped with Pitwall. The format, checks and build are
`pitwall_core::extensions::index`'s; this crate adds https downloads, the
headless run in Pitwall's tile runtime and the signatures.

| Command | What it does |
|---|---|
| `pitwall-index check <repo> [--base <dir>] [--login <github>] [--previous <index.json>] [--output <file>]` | Every file checked; the change judged against the base; each new version downloaded (size, SHA-256, the author's signature, package rules, licence, manifest = entry) and loaded headless with no grants. A Markdown report; exit 1 on a problem; `auto-merge=true` when the bot may merge. |
| `pitwall-index build <repo> --key <index.key> [--out <dir>]` | `dist/v1/index.json` and `blocked.json` built and signed, new screenshots under `screens/`, and each listing's README, icon, screenshots and README images under `media/<id>/` (named by SHA-256). Versions already in the old `index.json` aren't downloaded again. |
| `pitwall-index review <repo> <id> <version> --by <handle> --key <review.key> --reproducible` | A maintainer's review recorded in the entry: Verified. |
| `pitwall-index categories` | `categories.toml` as Pitwall knows it. |

`template/` is the index repository's starting content: the workflows
(`check`, `publish`, `auto-merge`), the report issue form, `SECURITY.md`,
`categories.toml` and its README. The owner's checklist for creating the
repository is in marketplace.md §7.8.

Tests run against a local fixture index (`pitwall_core::testing::index`),
never the network.
