# Pitwall extensions

The list of tiles Pitwall's Settings → Extensions → Browse shows. Each
extension is one file in `extensions/`, changed by pull request. Packages
stay on their authors' GitHub releases; this repository pins each by its
SHA-256 and the author's signature.

Listing is not endorsement. **Verified** means a maintainer read that
version's source, rebuilt it to the same component and signed a review.

## Submitting

1. Release your tile on GitHub: the `.pwtile` (`pitwall extension pack`)
   and its `.minisig` (`pitwall extension sign`) as assets.
2. Run `pitwall extension publish <file>.pwtile --repository <url> --author <handle> --github <login> --category <id>`.
   It writes `extensions/<id>.toml` (and `authors/<handle>.toml` the first
   time) and prints a link that opens a pull request here with them.
3. The `check` workflow downloads your release and checks it: the
   SHA-256, your signature, the package rules, a licence, and that each
   tile starts headless with no permissions. Fix what it reports.
4. A maintainer merges a new extension after a light look (the name isn't
   misleading or someone else's, a licence is there). An update by you
   that changes no permission, host or key is merged by the bot.

## Withdrawing and reporting

- Set `yanked = true` on a version to withdraw it: it isn't offered for
  new installs or updates; installed copies keep working.
- To report a problem with an extension, use "Report a problem" on its
  page in Pitwall, or open a "Report an extension" issue here. Security
  problems: see SECURITY.md.

## Files

- `extensions/<id>.toml`: id, author, repository, categories, licence,
  `[[versions]]` (version, url, sha256, commit, world, pitwall, yanked).
- `authors/<handle>.toml`: handle, GitHub login, public keys, contact.
- `maintainers.toml`: who reviews, with their review keys.
- `categories.toml`: the fixed list (it changes with Pitwall).
- `blocked.toml`: takedowns, by maintainers.
- `dist/v1/`: built and signed by the `publish` workflow. Never edited by
  hand.
