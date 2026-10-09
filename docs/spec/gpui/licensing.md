# GPUI port: version and licensing

Pitwall is Apache-2.0. The GPUI app may only use permissively licensed
crates (MIT, Apache-2.0, BSD, ISC, Zlib, CC0, Unicode and the like; MPL-2.0
only as a build tool, never linked). The rules:

- From Zed, only the `gpui` crate and its own sub-crates published with it
  (`gpui_*`, `gpui-macros`, all Apache-2.0). Zed's other crates (`ui`,
  `theme`, `editor`, `terminal`, `terminal_view`, `workspace`, `picker`,
  `menu`, `settings`, …) are **GPL-3.0**: never a dependency, never a source
  to copy or paraphrase. What they would provide is built in-house
  ([in-house.md](in-house.md)).
- Every new dependency is checked before it lands, and this file is
  updated (the table at the end is generated from `cargo metadata`; see
  "Re-checking").

## The GPUI version

**Choice: `gpui = "=0.2.2"` from crates.io** (published by Zed's crates.io
team, 2025-10-22, Apache-2.0), pinned exactly in
`crates/pitwall-app/Cargo.toml`.

- It is the newest `gpui` release on crates.io. A crates.io release means
  reproducible builds, no git dependency and a published licence per crate.
- `pitwall-term-view` (built in parallel) pins the same `=0.2.2`. Two GPUI
  versions in one app are impossible (their types differ), so the two
  crates always move together.
- It is a year behind Zed's main branch. Alternatives, if 0.2.2 blocks
  something:
  1. **A pinned Zed git rev**, using only `crates/gpui` (and the Apache-2.0
     sub-crates it needs) via `gpui = { git = "https://github.com/zed-industries/zed", rev = "<sha>" }`.
     Cargo resolves only the crates gpui depends on; still check that no GPL
     crate enters `Cargo.lock`.
  2. **`gpui-pre`** (Apache-2.0): weekly crates.io snapshots of Zed's gpui
     (0.3.8 = zed@279fe07, 2026-10-05), published by a third party (the
     `gpui-component` maintainer), not by Zed. Newer API, but an unofficial
     channel; would need its own review of each snapshot.
  Decision point: start of phase 1 and phase 2 (text input, terminal),
  together with `pitwall-term-view`.
- gpui's `runtime_shaders` feature is on by default in `pitwall-app` (no
  Metal toolchain needed to build; [packaging.md](packaging.md)).

## gpui-component

`gpui-component` (Longbridge, Apache-2.0) is a component library for GPUI.

| Release | GPUI it builds on | Usable with the pin |
|---|---|---|
| 0.3.x – 0.5.1 (Oct 2025 – Feb 2026) | `gpui ^0.2.2` (crates.io) | yes |
| 0.6.0 – 0.7.1 (Sep – Oct 2026) | `gpui-pre ^0.3` | no (would bring a second GPUI) |

Not used in phase 0. Phase 1 may take components from 0.5.1 after checking
its full dependency tree the same way (it pulls tree-sitter grammars,
`markdown`, `lsp-types`, … some optional).

## Candidate crates named in the spec

| Crate | Licence | For |
|---|---|---|
| alacritty_terminal 0.26 | Apache-2.0 | `pitwall-term-view` |
| futures 0.3 | MIT OR Apache-2.0 | the event bridge (added in phase 0) |
| cargo-packager 0.11 (tool) | Apache-2.0 OR MIT | bundles |
| tray-icon 0.26 | MIT OR Apache-2.0 | Windows tray (alternative to in-house) |
| notify-rust 4 | MIT OR Apache-2.0 | Linux notifications |
| zbus 5 | MIT | D-Bus (notifications, launcher badge) |
| accesskit 0.25 | MIT OR Apache-2.0 | accessibility bridge |
| tree-sitter 0.27 | MIT | syntax highlighting (grammars checked one by one) |
| syntect 5 | MIT | syntax highlighting alternative |
| raw-window-handle 0.6 | MIT OR Apache-2.0 OR Zlib | native window handles |
| nucleo-matcher | MPL-2.0 | **avoided** (fuzzy matching is written in-house) |

## What gpui 0.2.2 brought in (phase 0)

Adding `pitwall-app` added 395 crates (all targets: macOS, Linux,
Windows) that the Tauri app did not already use. All are permissive.
Points worth knowing:

- `cbindgen` (MPL-2.0): build dependency of gpui on macOS (generates shader
  bindings at build time). Not linked into the app; MPL-2.0 file-level
  copyleft does not reach Pitwall's code.
- `self_cell` (Apache-2.0 OR GPL-2.0-only): used under Apache-2.0.
- `ring` (Apache-2.0 AND ISC), `encoding_rs` ((Apache-2.0 OR MIT) AND
  BSD-3-Clause): permissive; their notices go into the bundle's licence file
  with the others in phase 8.
- `hexf-parse`, `tiny-keccak` (CC0-1.0): public domain dedication.
- Zed's republished forks (`zed-font-kit`, `zed-reqwest`, `zed-scap`,
  `zed-xim`, `zed-async-tar`) keep their upstreams' MIT/Apache-2.0.
- No crate is GPL, LGPL or AGPL-only.
- The shared `Cargo.lock` moved `core-foundation` from 0.10.1 to 0.10.0 for
  the Tauri app too (gpui pins `=0.10.0`), and `toml` 0.8.2 to 0.8.23.

Fonts (Inter, Barlow Condensed, JetBrains Mono, OFL-1.1) and the Material
Icon Theme subset (MIT) are already in `LICENSES/`.

## Re-checking

```sh
# licences of everything pitwall-app links, per target
for t in aarch64-apple-darwin x86_64-unknown-linux-gnu x86_64-pc-windows-msvc; do
  cargo metadata --format-version 1 --filter-platform $t
done
```

Walk `resolve.nodes` from `pitwall-app` over normal and build edges, read
each package's `license`, and diff against the table below. Anything not
clearly permissive is discussed before it lands. A `cargo-deny` config is a
good follow-up once the Tauri app is gone.

## Appendix: new crates by licence (phase 0)

| Licence | Count | Crates |
|---|---|---|
| MIT OR Apache-2.0 | 157 | aes 0.8.4, ahash 0.8.12, aligned 0.4.3, as-raw-xcb-connection 1.0.1, as-slice 0.2.1, ash 0.38.0+1.3.281, ash-window 0.13.0, async-compression 0.4.50, backtrace 0.3.76, block-buffer 0.12.1, block-padding 0.3.3, bstr 1.13.1, bumpalo 3.20.3, cbc 0.1.2, chacha20 0.10.2, cipher 0.4.4, cocoa 0.26.0, cocoa-foundation 0.2.0, compression-codecs 0.4.45, compression-core 0.4.33, const-random 0.1.18, const-random-macro 0.1.16, core-foundation 0.9.4, core-graphics 0.24.0, core-graphics-types 0.1.3, core-graphics2 0.4.1, core-text 21.0.0, core-video 0.4.3, cosmic-text 0.14.2, cpufeatures 0.3.1, crossbeam-deque 0.8.8, crossbeam-epoch 0.9.21, crossbeam-queue 0.3.14, crypto-common 0.2.2, data-url 0.3.2, digest 0.11.3, dirs 4.0.0, dirs 5.0.1, dirs-sys 0.3.7, dirs-sys 0.4.1, either 1.19.0, euclid 0.22.14, font-types 0.12.6, futures 0.3.34, gif 0.14.2, gimli 0.32.3, git2 0.20.4, gpu-alloc 0.6.2, gpu-alloc-ash 0.7.1, gpu-alloc-types 0.3.1, half 2.7.1, hashbrown 0.14.5, hashbrown 0.15.5, hkdf 0.12.4, hmac 0.12.1, httparse 1.10.1, hybrid-array 0.4.15, image 0.25.10, image-webp 0.2.4, inout 0.1.4, inventory 0.3.25, io-surface 0.16.1, ipnet 2.12.2, itertools 0.13.0, itertools 0.14.0, kv-log-macro 1.0.7, libgit2-sys 0.18.8+1.9.7, libz-sys 1.1.29, lyon 1.0.19, lyon_algorithms 1.0.21, lyon_geom 1.0.19, lyon_path 1.0.19, lyon_tessellation 1.0.22, md-5 0.10.6, memmap2 0.9.11, metal 0.29.0, naga 25.0.1, num 0.4.3, num-bigint 0.4.8, num-complex 0.4.6, num-derive 0.4.2, num-integer 0.1.47, num-iter 0.1.46, num-rational 0.4.2, num_cpus 1.17.0, openssl-probe 0.2.1, paste 1.0.15, pastey 0.1.1, pathfinder_simd 0.5.6, pbkdf2 0.12.2, pin-utils 0.1.1, prettyplease 0.2.37, proc-macro-error-attr2 2.0.0, proc-macro-error2 2.0.1, profiling 1.0.18, profiling-procmacros 1.0.18, psm 0.1.32, quinn 0.11.12, quinn-proto 0.11.19, quinn-udp 0.5.16, rand 0.10.3, rand 0.8.8, rand_chacha 0.3.1, rand_pcg 0.10.2, rayon 1.12.0, rayon-core 1.13.0, read-fonts 0.41.0, roxmltree 0.20.0, rustls-pki-types 1.15.1, rustversion 1.0.23, security-framework 3.7.0, security-framework-sys 2.17.0, sha2 0.11.0, shlex 1.3.0, simdutf8 0.1.5, skrifa 0.44.0, smol_str 0.2.2, stacker 0.1.25, static_assertions 1.1.0, sys-locale 0.3.2, system-configuration 0.6.1, system-configuration-sys 0.6.0, tempfile 3.27.0, tokio-rustls 0.26.6, ttf-parser 0.20.0, ttf-parser 0.21.1, ttf-parser 0.25.1, unicase 2.10.0, unicode-bidi 0.3.18, unicode-script 0.5.8, utf-8 0.7.6, wasm-bindgen 0.2.129, wasm-bindgen-macro 0.2.129, wasm-bindgen-macro-support 0.2.129, wasm-bindgen-shared 0.2.129, weezl 0.1.12, windows 0.57.0, windows 0.61.3, windows-collections 0.2.0, windows-core 0.57.0, windows-core 0.61.2, windows-future 0.2.1, windows-implement 0.57.0, windows-interface 0.57.0, windows-link 0.1.3, windows-numerics 0.2.0, windows-registry 0.4.0, windows-registry 0.5.3, windows-result 0.1.2, windows-result 0.3.4, windows-strings 0.3.1, windows-strings 0.4.2, windows-threading 0.1.0, x11rb 0.13.2, x11rb-protocol 0.13.2, zed-font-kit 0.14.1-zed, zed-reqwest 0.12.15-zed |
| MIT | 105 | aligned-vec 0.6.4, arg_enum_proc_macro 0.3.4, ashpd 0.11.1, ashpd 0.12.3, async_zip 0.0.17, av-scenechange 0.14.1, blade-graphics 0.7.1, blade-macros 0.3.0, blade-util 0.3.0, block 0.1.6, built 0.8.1, calloop 0.13.0, calloop-wayland-source 0.3.0, cfg_aliases 0.2.2, color_quant 1.1.0, convert_case 0.4.0, core_maths 0.1.1, crunchy 0.2.4, deflate64 0.1.12, derive_more 0.99.20, dlib 0.5.3, equator 0.4.2, equator-macro 0.4.2, fax 0.2.7, float-cmp 0.9.0, float_next_after 1.0.0, fontconfig-parser 0.5.8, fontdb 0.16.2, fontdb 0.23.0, freetype-sys 0.20.1, grid 0.18.0, h2 0.4.20, hidden-trait 0.1.2, http-body 1.1.0, http-body-util 0.1.5, hyper 1.11.1, hyper-util 0.1.21, imagesize 0.13.0, loop9 0.1.5, malloc_buf 0.0.6, maybe-rayon 0.1.1, mime_guess 2.0.5, mint 0.5.9, nix 0.29.0, nix 0.31.3, nom 7.1.3, nom 8.0.0, noop_proc_macro 0.3.0, objc 0.2.7, objc_exception 0.1.2, oo7 0.5.0, pico-args 0.5.0, postage 0.5.0, pulp 0.22.3, pulp-wasm-simd-flag 0.1.1, quick-xml 0.41.0, raw-cpuid 11.6.0, reborrow 0.5.5, rgb 0.8.53, rust-embed 8.13.0, rust-embed-impl 8.13.0, rust-embed-utils 8.13.0, rustybuzz 0.14.1, rustybuzz 0.20.1, schannel 0.1.29, seahash 4.1.0, simd_helpers 0.1.0, strict-num 0.1.1, strum 0.26.3, strum 0.27.2, strum_macros 0.26.4, strum_macros 0.27.2, sysinfo 0.31.4, taffy 0.9.0, take-until 0.2.0, tiff 0.11.3, tokio-socks 0.5.3, tokio-util 0.7.19, tower 0.5.3, tower-layer 0.3.3, tower-service 0.3.3, try-lock 0.2.5, want 0.3.2, wayland-backend 0.3.17, wayland-client 0.31.15, wayland-cursor 0.31.14, wayland-protocols 0.31.2, wayland-protocols 0.32.13, wayland-protocols-plasma 0.2.0, wayland-scanner 0.31.11, wayland-sys 0.31.11, which 6.0.3, windows-capture 1.5.0, winsafe 0.0.19, x11-clipboard 0.9.3, xcb 1.7.1, xcursor 0.3.11, xim-ctext 0.3.0, xim-parser 0.2.2, xkbcommon 0.8.0, xmlwriter 0.1.0, y4m 0.8.0, yeslogic-fontconfig-sys 6.0.1, zed-scap 0.0.8-zed, zed-xim 0.4.0-zed |
| Apache-2.0 OR MIT | 34 | addr2line 0.25.1, async-channel 1.9.0, async-fs 2.2.0, async-global-executor 2.4.1, async-net 2.0.0, async-std 1.13.2, const-oid 0.10.2, ctor 0.4.3, ctor-proc-macro 0.0.6, dtor 0.0.6, dtor-proc-macro 0.0.5, event-listener 2.5.3, fastrand 1.9.0, futures-lite 1.13.0, kurbo 0.11.3, leak 0.1.2, multiversion_no_op 1.0.0, no_std_io2 0.9.4, ntapi 0.4.3, object 0.37.3, object 0.39.1, pin-project 1.1.13, pin-project-internal 1.1.13, resvg 0.45.1, simplecss 0.2.2, smol 2.0.2, svgtypes 0.15.3, swash 0.2.10, usvg 0.45.1, waker-fn 1.2.0, yazi 0.2.1, zeno 0.3.3, zeroize 1.9.1, zeroize_derive 1.5.0 |
| MIT/Apache-2.0 | 26 | bitstream-io 4.10.0, core_detect 1.0.0, etagere 0.2.15, filetime 0.2.29, mac 0.1.1, minimal-lexical 0.2.1, num-bigint-dig 0.8.6, pathfinder_geometry 0.5.1, qoi 0.4.1, quick-error 2.0.1, rangemap 1.8.0, rustc-demangle 0.1.28, scoped-tls 1.0.1, serde_json_lenient 0.2.4, serde_urlencoded 0.7.1, svg_fmt 0.4.5, tendril 0.4.3, unicode-bidi-mirroring 0.2.0, unicode-bidi-mirroring 0.4.0, unicode-ccc 0.2.0, unicode-ccc 0.4.0, unicode-properties 0.1.4, unicode-vo 0.1.0, vcpkg 0.2.15, xattr 0.2.3, zed-async-tar 0.5.0-zed |
| Apache-2.0 | 21 | clang-sys 1.9.1, codespan-reporting 0.12.0, command-fds 0.3.3, gethostname 1.1.0, gpui 0.2.2, gpui-macros 0.2.2, gpui_collections 0.2.2, gpui_derive_refineable 0.2.2, gpui_http_client 0.2.2, gpui_media 0.2.2, gpui_perf 0.2.2, gpui_refineable 0.2.2, gpui_semantic_version 0.2.2, gpui_sum_tree 0.2.2, gpui_util 0.2.2, gpui_util_macros 0.2.2, spirv 0.3.0+sdk-1.3.268.0, stacksafe 0.1.4, stacksafe-macro 0.1.4, sync_wrapper 1.0.2, unicode-linebreak 0.1.5 |
| BSD-3-Clause | 7 | avif-serialize 0.8.9, bindgen 0.71.1, exr 1.74.2, lebe 0.5.3, ravif 0.13.0, tiny-skia 0.11.4, tiny-skia-path 0.11.4 |
| Apache-2.0/MIT | 6 | atomic 0.5.3, bit_field 0.10.3, cexpr 0.6.0, flume 0.11.1, pollster 0.2.5, rustc-hash 1.1.0 |
| MIT OR Apache-2.0 OR Zlib | 5 | lru-slab 0.1.3, xkeysym 0.2.1, zune-core 0.5.3, zune-inflate 0.2.54, zune-jpeg 0.5.15 |
| BSD-2-Clause | 4 | arrayref 0.3.9, av1-grain 0.2.5, rav1e 0.8.1, v_frame 0.3.9 |
| Apache-2.0 OR ISC OR MIT | 4 | hyper-rustls 0.27.10, rustls 0.23.45, rustls-native-certs 0.8.4, rustls-pemfile 2.2.0 |
| MIT / Apache-2.0 | 4 | cgl 0.3.2, float-ord 0.3.2, futf 0.1.5, leaky-cow 0.1.1 |
| ISC | 3 | libloading 0.8.9, rustls-webpki 0.103.15, untrusted 0.9.0 |
| Zlib | 3 | foldhash 0.1.5, nanorand 0.7.0, slotmap 1.1.1 |
| BSD-3-Clause OR Apache-2.0 | 2 | moxcms 0.8.1, pxfm 0.1.30 |
| Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 2 | linux-raw-sys 0.4.15, rustix 0.38.44 |
| Unlicense OR MIT | 2 | byteorder-lite 0.1.0, globset 0.4.20 |
| Zlib OR Apache-2.0 OR MIT | 2 | bytemuck 1.25.2, bytemuck_derive 1.12.1 |
| CC0-1.0 | 2 | hexf-parse 0.2.1, tiny-keccak 2.0.2 |
| (Apache-2.0 OR MIT) AND BSD-3-Clause | 1 | encoding_rs 0.8.42 |
| Apache-2.0 AND ISC | 1 | ring 0.17.14 |
| Apache-2.0 WITH LLVM-exception | 1 | ar_archive_writer 0.5.3 |
| MPL-2.0 | 1 | cbindgen 0.28.0 |
| CC0-1.0 OR Apache-2.0 | 1 | imgref 1.12.3 |
| Apache-2.0 OR GPL-2.0-only | 1 | self_cell 1.3.0 |

Main screen (`crates/pitwall-app/src/main_screen`): `regex` 1 (MIT OR Apache-2.0, already in the tree via pitwall-core) for New-agent name rules; embedded fonts `crates/pitwall-app/assets/fonts/` (Inter, Barlow Condensed, JetBrains Mono: OFL-1.1, `LICENSES/`), static TTF instances of the `@fontsource` files the web UI ships; icons are the paths of `src/components/Icon.tsx` (Pitwall's own).

## Added by Settings / appearance (phase 6)

All already in the tree (via gpui or the Tauri app); now direct dependencies of `pitwall-app`:

| Crate | Licence | For |
|---|---|---|
| raw-window-handle 0.6 | MIT OR Apache-2.0 OR Zlib | the window's native handle (Glass material) |
| objc2 0.6, objc2-foundation 0.3 (MIT); objc2-app-kit 0.3 (Zlib OR Apache-2.0 OR MIT) (macOS) | as listed | Liquid Glass (`NSGlassEffectView`), window appearance, accessibility options |
| windows-sys 0.61 (Windows) | MIT OR Apache-2.0 | Mica (`DwmSetWindowAttribute`), Reduce motion (`SystemParametersInfoW`) |

OS integration (phase 7, `src/platform/`): notify-rust 4.18 (MIT OR Apache-2.0), raw-window-handle 0.6 (MIT OR Apache-2.0 OR Zlib), objc2 0.6 / objc2-foundation 0.3 / objc2-app-kit 0.3 (MIT; macOS), tray-icon 0.25 (MIT OR Apache-2.0; Windows), windows 0.61 (MIT OR Apache-2.0; Windows), image 0.25 png-only (MIT OR Apache-2.0; Windows), zbus 5 (MIT; Linux). All were already in Cargo.lock (Tauri app, gpui): no new crates in the tree.

Packaging tools (phase 8; build-time only, nothing linked into Pitwall):
cargo-packager 0.11.8 (MIT OR Apache-2.0) for the Linux and Windows bundles,
and what it downloads while packaging: linuxdeploy and the AppImage runtime
(MIT), NSIS (zlib/libpng), WiX 3.11 (MS-RL, covers the toolset, not the .msi it
builds). macOS bundles use Apple's own tools (codesign, hdiutil, lipo).

Integration: `crates/pitwall-app/src/kit/` is the one component kit and asset source. Bundled fonts gain JetBrains Mono Bold, Italic and Bold Italic (OFL-1.1, `LICENSES/`), static instances of the `@fontsource-variable/jetbrains-mono` latin files, so terminal bold and italic match xterm. No new crates.
## Review and the code/diff viewer (gpui/review)

New crates in `pitwall-app` (`src/code_view`, `src/review`); all permissive,
none GPL. The tree-sitter grammars compile their C parsers with `cc` (already
in the tree); their highlight queries ship in the same crates under the same
licence.

| Crate | Licence | For |
|---|---|---|
| similar 2.7 | Apache-2.0 | line and character diff (Myers, with a deadline) |
| tree-sitter 0.25, tree-sitter-highlight 0.25, tree-sitter-language 0.1 | MIT | syntax highlighting |
| streaming-iterator 0.1 | MIT OR Apache-2.0 | dependency of tree-sitter |
| tree-sitter-{rust 0.24, javascript 0.25, typescript 0.23, python 0.25, go 0.25, bash 0.25, css 0.25, html 0.23, c 0.24, cpp 0.23, java 0.23, ruby 0.23, yaml 0.7} | MIT | the highlighted languages (JSON, TOML, diffs stay plain as in the React app; Markdown is not highlighted yet) |
| regex 1 | MIT OR Apache-2.0 | Find (already in the tree) |
| unicode-segmentation 1 | MIT OR Apache-2.0 | grapheme and word motion (already in the tree) |
| chrono 0.4 (`clock` only) | MIT OR Apache-2.0 | task times "HH:MM" (already in the tree) |
| libc 0.2 (`< 0.2.190`, Linux only) | MIT OR Apache-2.0 | not used directly: a version cap so gpui keeps compiling on Linux (already in the tree) |
| wayland-client 0.31 (Linux only) | MIT | asking the Wayland compositor whether it draws window decorations (already in the tree via gpui) |

The Material Icon Theme subset (MIT) is embedded from `src/assets/file-icons/`
for Review's file rows (the explorer embeds the same set; one copy should move
to the shared kit).
Fonts bundled by the Wall (`crates/pitwall-app/assets/fonts/`, loaded with `text_system().add_fonts`; to be promoted to the shared kit): Inter (Regular, Medium, SemiBold, Bold), Barlow Condensed (SemiBold, Bold) and JetBrains Mono (Regular, SemiBold), static latin instances of the fonts the React app ships via @fontsource; all SIL OFL-1.1, licence texts already in `LICENSES/` (OFL-1.1-inter.txt, OFL-1.1-barlow-condensed.txt, OFL-1.1-jetbrains-mono.txt).

Polish round (`kit/fonts.rs`): two more static instances of the same Inter variable font, weight 550 (`Inter-MediumPlus.ttf`) and 650 (`Inter-SemiBoldPlus.ttf`), for the CSS weights between the named ones; OFL-1.1 like the others (no new licence).

Parity round (`code_view/markdown.rs`): `tree-sitter-md` 0.5 (MIT, tree-sitter-grammars), the Markdown block and inline grammars for Review's Markdown colours (used through `tree-sitter-language`, no new tree-sitter version).

Parity round (`code_view/highlight.rs`, more of the languages React gets from `@codemirror/language-data`): tree-sitter grammars `tree-sitter-php` 0.25, `tree-sitter-c-sharp` 0.23, `tree-sitter-lua` 0.5, `tree-sitter-swift` 0.7, `tree-sitter-kotlin-ng` 1.1 (highlight query written in-house: `code_view/kotlin_highlights.scm`), `tree-sitter-scala` 0.26, `tree-sitter-sequel` 0.3 (SQL), `tree-sitter-xml` 0.7, `tree-sitter-haskell` 0.24, `tree-sitter-zig` 1.1, `tree-sitter-r` 1.3, `tree-sitter-powershell` 0.26, `tree-sitter-scss` 1.0, `tree-sitter-ocaml` 0.26, `tree-sitter-nix` 0.3 (all MIT) and `tree-sitter-elixir` 0.3 (Apache-2.0); all on the one tree-sitter 0.25.
