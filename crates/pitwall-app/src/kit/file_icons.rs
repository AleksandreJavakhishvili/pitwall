//! VS Code-style file and folder icons (Material Icon Theme subset, MIT,
//! see NOTICE): the same SVGs and name map as the React app
//! (`src/lib/fileIcons.ts`, `src/lib/fileIcons.map.json`,
//! `src/assets/file-icons/`), embedded in the binary and drawn in colour
//! (the explorer's tree, quick open, Review's file lists). Every coloured
//! SVG goes through [`svg_image`], which carries gpui 0.2.2's red/blue fix.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{img, px, Image, ImageFormat, Img, Styled};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Map {
    file: String,
    folder: [String; 2],
    file_names: HashMap<String, String>,
    extensions: HashMap<String, String>,
    folders: HashMap<String, [String; 2]>,
}

fn map() -> &'static Map {
    static MAP: OnceLock<Map> = OnceLock::new();
    MAP.get_or_init(|| {
        serde_json::from_str(include_str!("../../../../src/lib/fileIcons.map.json"))
            .expect("fileIcons.map.json")
    })
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Icon name for a file: exact file name, then the longest known
/// extension, then the generic one.
pub fn file_icon_name(path: &str) -> &'static str {
    let m = map();
    let name = base_name(path).to_lowercase();
    if let Some(n) = m.file_names.get(&name) {
        return n;
    }
    for (i, _) in name.match_indices('.') {
        let ext = &name[i + 1..];
        if let Some(n) = m.extensions.get(ext).filter(|_| !ext.is_empty()) {
            return n;
        }
    }
    &m.file
}

/// Icon name for a folder (by its last segment), open or closed.
pub fn folder_icon_name(path: &str, open: bool) -> &'static str {
    let m = map();
    let pair = m
        .folders
        .get(&base_name(path).to_lowercase())
        .unwrap_or(&m.folder);
    &pair[usize::from(open)]
}

fn svg(name: &str) -> Option<&'static [u8]> {
    SVGS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

fn image(name: &str) -> Arc<Image> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Image>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry(name.to_string())
        .or_insert_with(|| {
            let bytes = svg(name).or_else(|| svg(&map().file)).unwrap_or_default();
            svg_image(&String::from_utf8_lossy(bytes))
        })
        .clone()
}

/// `#rrggbb` / `#rgb` colours with red and blue swapped. gpui 0.2.2 hands
/// a rendered SVG's RGBA pixels to a BGRA texture without converting them
/// (other image formats are converted), so colours come out with red and
/// blue exchanged; swapping them in the source first cancels that out.
pub fn swap_red_blue(svg: &str) -> String {
    let b = svg.as_bytes();
    let mut out = String::with_capacity(svg.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            let run = b[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            let next_ok = b
                .get(i + 1 + run)
                .is_none_or(|c| !c.is_ascii_alphanumeric());
            if next_ok && (run == 6 || run == 3) {
                let h = &svg[i + 1..i + 1 + run];
                let n = run / 3;
                out.push('#');
                out.push_str(&h[2 * n..3 * n]);
                out.push_str(&h[n..2 * n]);
                out.push_str(&h[..n]);
                i += 1 + run;
                continue;
            }
        }
        let c = svg[i..].chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// A coloured SVG as an image for `img()`, colours fixed for gpui 0.2.2
/// ([`swap_red_blue`]). Cache the result: each call decodes again.
pub fn svg_image(svg: &str) -> Arc<Image> {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        swap_red_blue(svg).into_bytes(),
    ))
}

/// A file's icon, `size` points square.
pub fn file_icon(path: &str, size: f32) -> Img {
    img(image(file_icon_name(path))).size(px(size)).flex_none()
}

/// A folder's icon, open or closed.
pub fn folder_icon(path: &str, open: bool, size: f32) -> Img {
    img(image(folder_icon_name(path, open)))
        .size(px(size))
        .flex_none()
}

const SVGS: &[(&str, &[u8])] = &[
    (
        "agent",
        include_bytes!("../../../../src/assets/file-icons/agent.svg"),
    ),
    (
        "astro",
        include_bytes!("../../../../src/assets/file-icons/astro.svg"),
    ),
    (
        "c",
        include_bytes!("../../../../src/assets/file-icons/c.svg"),
    ),
    (
        "changelog",
        include_bytes!("../../../../src/assets/file-icons/changelog.svg"),
    ),
    (
        "console",
        include_bytes!("../../../../src/assets/file-icons/console.svg"),
    ),
    (
        "contributing",
        include_bytes!("../../../../src/assets/file-icons/contributing.svg"),
    ),
    (
        "cpp",
        include_bytes!("../../../../src/assets/file-icons/cpp.svg"),
    ),
    (
        "csharp",
        include_bytes!("../../../../src/assets/file-icons/csharp.svg"),
    ),
    (
        "css",
        include_bytes!("../../../../src/assets/file-icons/css.svg"),
    ),
    (
        "dart",
        include_bytes!("../../../../src/assets/file-icons/dart.svg"),
    ),
    (
        "database",
        include_bytes!("../../../../src/assets/file-icons/database.svg"),
    ),
    (
        "diff",
        include_bytes!("../../../../src/assets/file-icons/diff.svg"),
    ),
    (
        "docker",
        include_bytes!("../../../../src/assets/file-icons/docker.svg"),
    ),
    (
        "document",
        include_bytes!("../../../../src/assets/file-icons/document.svg"),
    ),
    (
        "elixir",
        include_bytes!("../../../../src/assets/file-icons/elixir.svg"),
    ),
    (
        "eslint",
        include_bytes!("../../../../src/assets/file-icons/eslint.svg"),
    ),
    (
        "favicon",
        include_bytes!("../../../../src/assets/file-icons/favicon.svg"),
    ),
    (
        "file",
        include_bytes!("../../../../src/assets/file-icons/file.svg"),
    ),
    (
        "folder",
        include_bytes!("../../../../src/assets/file-icons/folder.svg"),
    ),
    (
        "folder-api",
        include_bytes!("../../../../src/assets/file-icons/folder-api.svg"),
    ),
    (
        "folder-api-open",
        include_bytes!("../../../../src/assets/file-icons/folder-api-open.svg"),
    ),
    (
        "folder-app",
        include_bytes!("../../../../src/assets/file-icons/folder-app.svg"),
    ),
    (
        "folder-app-open",
        include_bytes!("../../../../src/assets/file-icons/folder-app-open.svg"),
    ),
    (
        "folder-components",
        include_bytes!("../../../../src/assets/file-icons/folder-components.svg"),
    ),
    (
        "folder-components-open",
        include_bytes!("../../../../src/assets/file-icons/folder-components-open.svg"),
    ),
    (
        "folder-config",
        include_bytes!("../../../../src/assets/file-icons/folder-config.svg"),
    ),
    (
        "folder-config-open",
        include_bytes!("../../../../src/assets/file-icons/folder-config-open.svg"),
    ),
    (
        "folder-css",
        include_bytes!("../../../../src/assets/file-icons/folder-css.svg"),
    ),
    (
        "folder-css-open",
        include_bytes!("../../../../src/assets/file-icons/folder-css-open.svg"),
    ),
    (
        "folder-dist",
        include_bytes!("../../../../src/assets/file-icons/folder-dist.svg"),
    ),
    (
        "folder-dist-open",
        include_bytes!("../../../../src/assets/file-icons/folder-dist-open.svg"),
    ),
    (
        "folder-docs",
        include_bytes!("../../../../src/assets/file-icons/folder-docs.svg"),
    ),
    (
        "folder-docs-open",
        include_bytes!("../../../../src/assets/file-icons/folder-docs-open.svg"),
    ),
    (
        "folder-github",
        include_bytes!("../../../../src/assets/file-icons/folder-github.svg"),
    ),
    (
        "folder-github-open",
        include_bytes!("../../../../src/assets/file-icons/folder-github-open.svg"),
    ),
    (
        "folder-hook",
        include_bytes!("../../../../src/assets/file-icons/folder-hook.svg"),
    ),
    (
        "folder-hook-open",
        include_bytes!("../../../../src/assets/file-icons/folder-hook-open.svg"),
    ),
    (
        "folder-images",
        include_bytes!("../../../../src/assets/file-icons/folder-images.svg"),
    ),
    (
        "folder-images-open",
        include_bytes!("../../../../src/assets/file-icons/folder-images-open.svg"),
    ),
    (
        "folder-lib",
        include_bytes!("../../../../src/assets/file-icons/folder-lib.svg"),
    ),
    (
        "folder-lib-open",
        include_bytes!("../../../../src/assets/file-icons/folder-lib-open.svg"),
    ),
    (
        "folder-open",
        include_bytes!("../../../../src/assets/file-icons/folder-open.svg"),
    ),
    (
        "folder-public",
        include_bytes!("../../../../src/assets/file-icons/folder-public.svg"),
    ),
    (
        "folder-public-open",
        include_bytes!("../../../../src/assets/file-icons/folder-public-open.svg"),
    ),
    (
        "folder-resource",
        include_bytes!("../../../../src/assets/file-icons/folder-resource.svg"),
    ),
    (
        "folder-resource-open",
        include_bytes!("../../../../src/assets/file-icons/folder-resource-open.svg"),
    ),
    (
        "folder-scripts",
        include_bytes!("../../../../src/assets/file-icons/folder-scripts.svg"),
    ),
    (
        "folder-scripts-open",
        include_bytes!("../../../../src/assets/file-icons/folder-scripts-open.svg"),
    ),
    (
        "folder-server",
        include_bytes!("../../../../src/assets/file-icons/folder-server.svg"),
    ),
    (
        "folder-server-open",
        include_bytes!("../../../../src/assets/file-icons/folder-server-open.svg"),
    ),
    (
        "folder-src",
        include_bytes!("../../../../src/assets/file-icons/folder-src.svg"),
    ),
    (
        "folder-src-open",
        include_bytes!("../../../../src/assets/file-icons/folder-src-open.svg"),
    ),
    (
        "folder-src-tauri",
        include_bytes!("../../../../src/assets/file-icons/folder-src-tauri.svg"),
    ),
    (
        "folder-src-tauri-open",
        include_bytes!("../../../../src/assets/file-icons/folder-src-tauri-open.svg"),
    ),
    (
        "folder-store",
        include_bytes!("../../../../src/assets/file-icons/folder-store.svg"),
    ),
    (
        "folder-store-open",
        include_bytes!("../../../../src/assets/file-icons/folder-store-open.svg"),
    ),
    (
        "folder-test",
        include_bytes!("../../../../src/assets/file-icons/folder-test.svg"),
    ),
    (
        "folder-test-open",
        include_bytes!("../../../../src/assets/file-icons/folder-test-open.svg"),
    ),
    (
        "folder-typescript",
        include_bytes!("../../../../src/assets/file-icons/folder-typescript.svg"),
    ),
    (
        "folder-typescript-open",
        include_bytes!("../../../../src/assets/file-icons/folder-typescript-open.svg"),
    ),
    (
        "folder-utils",
        include_bytes!("../../../../src/assets/file-icons/folder-utils.svg"),
    ),
    (
        "folder-utils-open",
        include_bytes!("../../../../src/assets/file-icons/folder-utils-open.svg"),
    ),
    (
        "folder-vscode",
        include_bytes!("../../../../src/assets/file-icons/folder-vscode.svg"),
    ),
    (
        "folder-vscode-open",
        include_bytes!("../../../../src/assets/file-icons/folder-vscode-open.svg"),
    ),
    (
        "font",
        include_bytes!("../../../../src/assets/file-icons/font.svg"),
    ),
    (
        "gemfile",
        include_bytes!("../../../../src/assets/file-icons/gemfile.svg"),
    ),
    (
        "git",
        include_bytes!("../../../../src/assets/file-icons/git.svg"),
    ),
    (
        "go",
        include_bytes!("../../../../src/assets/file-icons/go.svg"),
    ),
    (
        "go-mod",
        include_bytes!("../../../../src/assets/file-icons/go-mod.svg"),
    ),
    (
        "graphql",
        include_bytes!("../../../../src/assets/file-icons/graphql.svg"),
    ),
    (
        "h",
        include_bytes!("../../../../src/assets/file-icons/h.svg"),
    ),
    (
        "hcl",
        include_bytes!("../../../../src/assets/file-icons/hcl.svg"),
    ),
    (
        "hpp",
        include_bytes!("../../../../src/assets/file-icons/hpp.svg"),
    ),
    (
        "html",
        include_bytes!("../../../../src/assets/file-icons/html.svg"),
    ),
    (
        "http",
        include_bytes!("../../../../src/assets/file-icons/http.svg"),
    ),
    (
        "image",
        include_bytes!("../../../../src/assets/file-icons/image.svg"),
    ),
    (
        "java",
        include_bytes!("../../../../src/assets/file-icons/java.svg"),
    ),
    (
        "javascript",
        include_bytes!("../../../../src/assets/file-icons/javascript.svg"),
    ),
    (
        "jsconfig",
        include_bytes!("../../../../src/assets/file-icons/jsconfig.svg"),
    ),
    (
        "json",
        include_bytes!("../../../../src/assets/file-icons/json.svg"),
    ),
    (
        "jupyter",
        include_bytes!("../../../../src/assets/file-icons/jupyter.svg"),
    ),
    (
        "kotlin",
        include_bytes!("../../../../src/assets/file-icons/kotlin.svg"),
    ),
    (
        "less",
        include_bytes!("../../../../src/assets/file-icons/less.svg"),
    ),
    (
        "license",
        include_bytes!("../../../../src/assets/file-icons/license.svg"),
    ),
    (
        "lock",
        include_bytes!("../../../../src/assets/file-icons/lock.svg"),
    ),
    (
        "log",
        include_bytes!("../../../../src/assets/file-icons/log.svg"),
    ),
    (
        "lua",
        include_bytes!("../../../../src/assets/file-icons/lua.svg"),
    ),
    (
        "makefile",
        include_bytes!("../../../../src/assets/file-icons/makefile.svg"),
    ),
    (
        "markdown",
        include_bytes!("../../../../src/assets/file-icons/markdown.svg"),
    ),
    (
        "mdx",
        include_bytes!("../../../../src/assets/file-icons/mdx.svg"),
    ),
    (
        "nix",
        include_bytes!("../../../../src/assets/file-icons/nix.svg"),
    ),
    (
        "nodejs",
        include_bytes!("../../../../src/assets/file-icons/nodejs.svg"),
    ),
    (
        "npm",
        include_bytes!("../../../../src/assets/file-icons/npm.svg"),
    ),
    (
        "objective-c",
        include_bytes!("../../../../src/assets/file-icons/objective-c.svg"),
    ),
    (
        "objective-cpp",
        include_bytes!("../../../../src/assets/file-icons/objective-cpp.svg"),
    ),
    (
        "pdf",
        include_bytes!("../../../../src/assets/file-icons/pdf.svg"),
    ),
    (
        "php",
        include_bytes!("../../../../src/assets/file-icons/php.svg"),
    ),
    (
        "pnpm",
        include_bytes!("../../../../src/assets/file-icons/pnpm.svg"),
    ),
    (
        "powershell",
        include_bytes!("../../../../src/assets/file-icons/powershell.svg"),
    ),
    (
        "prettier",
        include_bytes!("../../../../src/assets/file-icons/prettier.svg"),
    ),
    (
        "proto",
        include_bytes!("../../../../src/assets/file-icons/proto.svg"),
    ),
    (
        "python",
        include_bytes!("../../../../src/assets/file-icons/python.svg"),
    ),
    (
        "python-misc",
        include_bytes!("../../../../src/assets/file-icons/python-misc.svg"),
    ),
    (
        "r",
        include_bytes!("../../../../src/assets/file-icons/r.svg"),
    ),
    (
        "react",
        include_bytes!("../../../../src/assets/file-icons/react.svg"),
    ),
    (
        "react_ts",
        include_bytes!("../../../../src/assets/file-icons/react_ts.svg"),
    ),
    (
        "readme",
        include_bytes!("../../../../src/assets/file-icons/readme.svg"),
    ),
    (
        "ruby",
        include_bytes!("../../../../src/assets/file-icons/ruby.svg"),
    ),
    (
        "rust",
        include_bytes!("../../../../src/assets/file-icons/rust.svg"),
    ),
    (
        "sass",
        include_bytes!("../../../../src/assets/file-icons/sass.svg"),
    ),
    (
        "settings",
        include_bytes!("../../../../src/assets/file-icons/settings.svg"),
    ),
    (
        "svelte",
        include_bytes!("../../../../src/assets/file-icons/svelte.svg"),
    ),
    (
        "svg",
        include_bytes!("../../../../src/assets/file-icons/svg.svg"),
    ),
    (
        "swift",
        include_bytes!("../../../../src/assets/file-icons/swift.svg"),
    ),
    (
        "table",
        include_bytes!("../../../../src/assets/file-icons/table.svg"),
    ),
    (
        "tailwindcss",
        include_bytes!("../../../../src/assets/file-icons/tailwindcss.svg"),
    ),
    (
        "tauri",
        include_bytes!("../../../../src/assets/file-icons/tauri.svg"),
    ),
    (
        "terraform",
        include_bytes!("../../../../src/assets/file-icons/terraform.svg"),
    ),
    (
        "test-js",
        include_bytes!("../../../../src/assets/file-icons/test-js.svg"),
    ),
    (
        "test-jsx",
        include_bytes!("../../../../src/assets/file-icons/test-jsx.svg"),
    ),
    (
        "test-ts",
        include_bytes!("../../../../src/assets/file-icons/test-ts.svg"),
    ),
    (
        "tex",
        include_bytes!("../../../../src/assets/file-icons/tex.svg"),
    ),
    (
        "toml",
        include_bytes!("../../../../src/assets/file-icons/toml.svg"),
    ),
    (
        "tsconfig",
        include_bytes!("../../../../src/assets/file-icons/tsconfig.svg"),
    ),
    (
        "tune",
        include_bytes!("../../../../src/assets/file-icons/tune.svg"),
    ),
    (
        "typescript",
        include_bytes!("../../../../src/assets/file-icons/typescript.svg"),
    ),
    (
        "typescript-def",
        include_bytes!("../../../../src/assets/file-icons/typescript-def.svg"),
    ),
    (
        "vite",
        include_bytes!("../../../../src/assets/file-icons/vite.svg"),
    ),
    (
        "vitest",
        include_bytes!("../../../../src/assets/file-icons/vitest.svg"),
    ),
    (
        "vue",
        include_bytes!("../../../../src/assets/file-icons/vue.svg"),
    ),
    (
        "webassembly",
        include_bytes!("../../../../src/assets/file-icons/webassembly.svg"),
    ),
    (
        "xml",
        include_bytes!("../../../../src/assets/file-icons/xml.svg"),
    ),
    (
        "yaml",
        include_bytes!("../../../../src/assets/file-icons/yaml.svg"),
    ),
    (
        "yarn",
        include_bytes!("../../../../src/assets/file-icons/yarn.svg"),
    ),
    (
        "zig",
        include_bytes!("../../../../src/assets/file-icons/zig.svg"),
    ),
    (
        "zip",
        include_bytes!("../../../../src/assets/file-icons/zip.svg"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pick_icons_like_the_react_app() {
        assert_eq!(file_icon_name("package.json"), "nodejs");
        assert_eq!(file_icon_name("src/main.rs"), "rust");
        assert_eq!(file_icon_name("a/b/unknown.zzz"), "file");
        assert_eq!(folder_icon_name("src", false), map().folders["src"][0]);
        assert_eq!(folder_icon_name("whatever", true), "folder-open");
    }

    #[test]
    fn red_and_blue_swap_in_svg_colours() {
        assert_eq!(
            swap_red_blue(r##"<p fill="#ff7043"/><p fill="#a0f"/><p fill="url(#a)"/>"##),
            r##"<p fill="#4370ff"/><p fill="#f0a"/><p fill="url(#a)"/>"##
        );
    }

    #[test]
    fn every_mapped_icon_is_embedded() {
        let m = map();
        let all = m
            .file_names
            .values()
            .chain(m.extensions.values())
            .chain(m.folders.values().flatten())
            .chain(m.folder.iter())
            .chain(std::iter::once(&m.file));
        for name in all {
            assert!(svg(name).is_some(), "{name}.svg is not embedded");
        }
    }
}
