# Publishing

**Preview.** How to list your tile in the extensions index, so that people find it in Settings → Extensions → Browse. [Marketplace](marketplace.md) covers what the index is and what Verified means. The index isn't open yet: until the outside security review is done, it takes tiles only from authors on the SDK preview.

You need a signed package ([Packaging](packaging.md)) and a public GitHub repository with the tile's source.

## What the index needs

On top of what every package needs:

- **A `LICENSE` file** in the package, and its SPDX name (`MIT`, `Apache-2.0`…) in the entry.
- **A `"description"`** in `extension.json`: the one line Browse shows.
- **A GitHub release** of the tile's repository with the `.pwtile` and its `.minisig` as assets.
- **The commit** the component was built from, so that it can be rebuilt for Verified.
- **1 to 3 categories:** `git`, `ci`, `databases`, `apis`, `notes`, `monitoring`, `docs`, `fun`, `other`.

`pitwall extension pack` reminds you when the license or the description is missing.

## Release

```bash
pitwall extension dev --once
pitwall extension pack
pitwall extension sign my-tile-1.0.0.pwtile
```

Tag the commit you built from, then make a GitHub release with both files, `my-tile-1.0.0.pwtile` and `my-tile-1.0.0.pwtile.minisig`. Once a version is in the index, its file must never change. To fix something, release a new version.

## Publish

`pitwall extension publish` writes the index entry for the package and prints the link that opens the pull request. It never commits, pushes or uploads anything.

**The first time**, run it beside the package:

```bash
pitwall extension publish my-tile-1.0.0.pwtile \
  --repository https://github.com/you/pitwall-my-tile \
  --author you --category ci --license MIT
```

It writes `index-entry/extensions/my-tile.toml` and `index-entry/authors/you.toml`, the latter holding your public key. It then prints GitHub's new-file link, prefilled with the entry. Saving the file on GitHub makes a fork and opens the pull request. Add your author file to the same pull request.

**With a clone of the index** (your fork), give `--index-dir`. The entry is written there, or the new version is added to it, and the index's own checks run before you push:

```bash
pitwall extension publish my-tile-1.1.0.pwtile \
  --repository https://github.com/you/pitwall-my-tile --author you \
  --index-dir ~/src/pitwall-extensions
```

It prints the git commands to commit and push a branch, and a link that opens the pull request with its title filled in.

**An update without a clone:** add `--update`. It prints the `[[versions]]` block to add to your entry, and the link to edit the entry on GitHub.

By default the download address is `<repository>/releases/download/v<version>/<file>`, and the commit is the folder's `git rev-parse HEAD`. Use `--url` or `--commit` when yours differ. `--github` sets your GitHub login (by default, the repository's owner), and `--index` names another index repository.

## What the check does

The index's `check` workflow runs on your pull request, with no secrets and a read-only token:

1. Every file in the index is checked: ids and handles match their file names, versions are well formed, downloads are release assets of your repository, keys are valid.
2. The change is compared with the index as it was. An extension stays with its author. A listed version is never changed or removed: set `yanked = true` instead. A new version must be newer than the listed ones. Ids that look like an existing one (`ci-statu5` next to `ci-status`) are flagged for a maintainer.
3. Each new version is downloaded and checked: at most 50 MiB, its SHA-256 the one in the entry, its signature by one of your keys in `authors/<you>.toml`, the package rules, a licence and a description, and a manifest that matches the entry.
4. Each tile is loaded headless in Pitwall's tile runtime, with no permissions granted. It must start and draw a first frame.
5. Each new version is compared with the one before it: its permissions, hosts, memory and signing key.

The check posts its report as the job's summary.

## Who merges

- **A new extension:** a maintainer, after a light look. They check that the name isn't misleading or someone else's, and that a licence is there.
- **An update by you that changes no permission, host, memory or key:** the bot, once every check passes.
- **Anything else** (new permissions or hosts, a new key, another repository, a pull request by someone other than you): a maintainer.

## Withdraw a version

Set `yanked = true` on its `[[versions]]` entry in a pull request. It is no longer offered for new installs or updates, and installed copies keep working. Only a maintainer can undo it.

## Change your key

Add the new public key to `authors/<you>.toml` in a pull request; a maintainer checks that it is really you. Sign the next version with it. People who installed your tile are asked to trust the new key when they update ([Install](install.md#who-signed-it)). If your key was stolen, tell the maintainers privately (see the index's `SECURITY.md`) so the versions signed with it can be blocked.

## Verified

A maintainer may review a version for Verified ([Marketplace](marketplace.md#verified)). To make that easy, keep the source of each version tagged, make builds reproducible (the same `cargo build --release --target wasm32-wasip2` gives the same `tile.wasm`), and give accurate reasons for every permission. A review covers one version. An update that changes no permission, host or key is reviewed quickly. One that does goes back to a full review.
