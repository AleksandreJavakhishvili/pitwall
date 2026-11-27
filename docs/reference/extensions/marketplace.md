# Marketplace

**Preview.** The extensions index is the list of tiles that Settings → Extensions → Browse shows. This page covers what the index is, how Pitwall checks it, what Verified means, and what happens when a version is withdrawn, blocked or no longer listed. [Publishing](publishing.md) is the author's side.

The index isn't open yet. Until the [outside security review](#before-it-opens) is done, it lists only tiles by authors on the SDK preview.

## What the index is

The index is a public GitHub repository, `pitwalls/pitwall-extensions`. It holds one small file per extension: its author, its source repository, its categories, its licence, and each version's download address, SHA-256 and the commit it was built from. Changes come in as pull requests. A check runs on each one, then a maintainer, or for a plain update a bot, merges it.

The packages stay on their authors' GitHub releases. The index doesn't copy or rebuild them. It pins each one by its SHA-256 and by the author's signature.

On every change, and once a day, the index's CI builds `index.json` (the listings) and `blocked.json` (takedowns), and signs both with the index key.

## What Pitwall checks

Pitwall reads the two files at start and once a day. It sends a conditional request, so an unchanged index costs almost nothing. Before it uses or keeps either file, it checks:

- **The signature**, against the index keys built into Pitwall. A key named inside the file doesn't count. A file that doesn't check is refused, and Pitwall keeps the copy it had.
- **That it isn't older** than the copy Pitwall already has. An old index that was validly signed, served again, is refused too.
- **Strict limits.** Sizes and counts are bounded. Names and descriptions must be plain text, with no control characters and no characters that reverse text direction. Addresses must be https GitHub release assets of the extension's own repository. Hashes must be hex, and categories must come from the fixed list. Anything else rejects the whole file.

When Pitwall is offline, or the index server fails, Pitwall uses its cached copy and says why. A fetch that fails changes nothing.

When you install from the index, the package must match the listed SHA-256 and be signed by one of the author's keys. Then it goes through the same checks and install screen as a file ([Install](install.md)).

## Verified

**Verified** means a Pitwall maintainer read that version's source and found that it does what it says:

- the component, rebuilt from the tagged source, came out byte for byte the same;
- the source matches the permissions it asks for: the hosts it contacts, the files it touches, the text it queues to agents;
- each permission's reason is accurate;
- it has no obfuscated code and no bundled programs.

Verified is per version, not per author. The maintainer signs a short statement that names the extension, the version and the package's SHA-256. Pitwall checks that signature against review keys that are built into Pitwall, not against anything in the index. A stolen index key alone can't make a version Verified.

A new version isn't Verified until someone reviews it. Until then it shows as **Not yet reviewed**, and Install offers the last Verified version by default. Browse's "Verified only" filter is on by default.

Listing isn't endorsement: a tile that isn't Verified was checked by machines and glanced at by a maintainer, not reviewed.

## Withdrawn, blocked, no longer listed

| Mark | Who sets it | What Pitwall does |
|---|---|---|
| **Withdrawn** | The author, or a maintainer | The version isn't offered for new installs or updates. An installed copy keeps working. Its card says "1.0.0 was withdrawn by its author." |
| **Blocked** | Maintainers, for malware or abuse | Installs and updates of the version are refused, with the reason. An installed copy is turned off at the next daily check, with the reason, and Remove is offered. It can't be turned back on. |
| **No longer listed** | Maintainers (unmaintained, a licence problem) | It leaves Browse. An installed copy keeps working, but gets no more updates. |

A block can name a whole extension, some versions, or particular packages by their SHA-256.

## Report a problem

"Report a problem" on an extension's page opens a new issue on the index repository in your browser. The issue is filled in with the extension's id, version and package SHA-256, plus what is wrong: it does something harmful, it isn't what it says it is, it doesn't work, or something else. Pitwall sends nothing itself. You review the issue and submit it on GitHub.

To report something privately, such as a security problem, use the address in the index's `SECURITY.md`.

## Before it opens

Browse opens to everyone only after two things are done: a threat model of the marketplace, and a security review by someone outside the project. The threat model covers a compromised index, key rotation, look-alike names and malicious updates. It is written; the review hasn't started yet.
