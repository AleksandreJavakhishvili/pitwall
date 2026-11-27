# Packaging

**Preview.** How to turn a tile project into a `.pwtile` package, sign it, and pass it on. [Install](install.md) covers what the person who gets it sees.

A package is one file holding the tile and everything it needs. A signature file sits beside it. Pitwall checks the signature before it unpacks anything. The person who installs it then knows the file wasn't changed on the way, and that later versions come from the same author.

## Pack

Build the tile first, then pack it:

```bash
pitwall extension dev --once
pitwall extension pack
```

`pack` writes `<id>-<version>.pwtile` into the project folder, or to the path given with `-o`. It checks the component the way `pitwall extension check` does, then copies in these files and nothing else:

| File | |
|---|---|
| `extension.json` | Required. Its `version` is required too. |
| `tile.wasm` | Required. The component, at most 20 MiB. |
| `README.md` | Required. |
| `LICENSE`, `LICENSE.md`, `LICENSE.txt`, `NOTICE` | Optional. |
| `CHANGELOG.md` | Optional. |
| the input schemas its tiles name | Required when named. |
| `agents/SKILL.md` | Optional. An agent skill, text only, at most 16 KiB. |
| `assets/` | Optional. svg, png and webp only. Fonts are refused. |
| `screenshots/` | Optional. png or webp, at most 6, at most 2 MiB each. |

Sources, `target/` and hidden files stay out. A symbolic link stops the pack. File names may use only ASCII letters, digits, `.`, `_`, `-` and `/`. The whole package may be at most 50 MiB.

The same files always give the same package, byte for byte. Two people who build the same component and pack it get identical files.

A package holds tiles only. An extension that also runs a program (an adapter or a service) is still installed from Git, npm or a folder.

## Make a signing key

Make one key and use it for every version you publish. Use [minisign](https://jedisct1.github.io/minisign/) or [rsign2](https://github.com/jedisct1/rsign2):

```bash
minisign -G
```

This writes the secret key to `~/.minisign/minisign.key`, protected by a password you choose, and the public key to `minisign.pub` in the current folder. You may also leave the password empty.

- **Keep the secret key out of every repository**, including private ones. Don't put it in your project folder. Store a backup somewhere safe: if you lose the key, people who installed your tile must explicitly trust a new one.
- **Publish the public key** where people can find it and you control the page, such as your project's README or website. It is the line that starts with `RW` in `minisign.pub`. People compare it with the key Pitwall shows them.
- On a build server, keep the key in the server's secret store and give its password in `PITWALL_SIGN_PASSWORD`.

## Sign

```bash
pitwall extension sign my-tile-0.1.0.pwtile
```

`sign` writes `my-tile-0.1.0.pwtile.minisig` beside the package. It uses `~/.minisign/minisign.key`, or the key given with `--key`. A key with a password asks for it, or reads it from `PITWALL_SIGN_PASSWORD`. When it is done, it prints the key id.

The signature is in minisign's format, so `minisign -V` can check it. Its untrusted comment carries your public key, so Pitwall can check a first install without being given the key separately. A signature made with plain `minisign -S` also works, but only for updates: it has no public key in it, so Pitwall needs the key it already trusts for that extension.

The trusted comment names the file and the extension's id and version. It is signed, and `minisign -V` shows it.

## Pass it on

Send both files, the `.pwtile` and its `.minisig`, and keep them side by side. The person installs it from Settings → Extensions → Install from file…, or with `pitwall extension install <file>`.

## Updates

Pack and sign each new version with **the same key**, and raise `version` in `extension.json`. Pitwall refuses:

- a version signed by another key, unless the person explicitly trusts the new key;
- an older version (they use Go back for that instead);
- the version already installed.

When a version asks for new permissions, more memory, a larger database or new `net` hosts, Pitwall lists the changes before the update. It grants none of them until the person allows each one. Say why in each permission's reason, and in `CHANGELOG.md`.

If you must change keys, tell people first, through the same place where you published the old key. Pitwall will show them the new key and ask whether to trust it.

## Folders still work

While you develop, use `pitwall extension dev`. To share a quick preview without a key, a folder works as before ([Quick start](quick-start.md#share-a-preview-tile)), and so does an unsigned package. Both are marked as previews. Neither can replace a signed install.

## What Pitwall checks

When it opens a package, Pitwall checks it in this order and stops at the first problem:

1. The file is at most 50 MiB.
2. The signature matches the file's bytes, and the key is the one trusted for this extension, if one is.
3. The zip is plain: no zip64, encryption or comment, stored or deflate only, and headers that agree with each other.
4. Every name passes the rules above. There are no duplicates (ignoring case), no `..`, no absolute paths, and no links or special files.
5. Every size is within its limit before anything is inflated, and each file's checksum matches.
6. The manifest is valid, its files are the ones allowed, and `tile.wasm` is a component that needs only what this Pitwall provides.

Only then is the package unpacked, into a new hidden folder, and shown to the person.
