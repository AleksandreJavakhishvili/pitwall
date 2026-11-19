//! One file for the viewer: text, binary or too large, with a language hint.

use pitwall_proto::{ContentKind, FileView};

use super::Res;
use crate::exec::{Exec, FileKind};

/// Text files up to this size are sent whole.
pub const MAX_TEXT: u64 = 2 * 1024 * 1024;
/// The cap when the user asks for a larger file anyway ("Load anyway").
pub const MAX_LARGE: u64 = 10 * 1024 * 1024;
/// Images the viewer draws are read up to this size; larger ones are
/// binary.
pub const MAX_IMAGE: u64 = 10 * 1024 * 1024;
/// PDFs the viewer draws (where it can) are read up to this size.
pub const MAX_PDF: u64 = 64 * 1024 * 1024;
/// A NUL in this many first bytes makes a file binary (git's rule).
const SNIFF: usize = 8000;

/// Extensions that are binary whatever their first bytes say; their
/// contents are never read.
const BINARY_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "icns", "tif", "tiff", "psd", "heic",
    "avif", "pdf", "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "tar", "jar", "war",
    "class", "o", "a", "so", "dylib", "dll", "exe", "bin", "wasm", "woff", "woff2", "ttf", "otf",
    "eot", "mp3", "mp4", "m4a", "mov", "avi", "mkv", "webm", "wav", "flac", "ogg", "sqlite", "db",
    "pyc",
];

/// Images the viewer draws (contents read separately, [`read_bytes`]).
pub const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];

/// `name` is an image the viewer draws.
pub fn is_image(name: &str) -> bool {
    ext(name).is_some_and(|e| IMAGE_EXT.contains(&e.as_str()))
}

/// `name` is an SVG: an image whose text is also sent (its source can be
/// read and edited as XML).
pub fn is_svg(name: &str) -> bool {
    ext(name).is_some_and(|e| e == "svg")
}

/// `name` is a PDF. Still binary to [`read`] (its bytes go to the viewer
/// only where it can draw PDFs, through [`read_bytes`]).
pub fn is_pdf(name: &str) -> bool {
    ext(name).is_some_and(|e| e == "pdf")
}

/// The most [`read_bytes`] reads of `name` for the viewer: [`MAX_PDF`] for
/// a PDF, else [`MAX_IMAGE`].
pub fn bytes_cap(name: &str) -> u64 {
    if is_pdf(name) {
        MAX_PDF
    } else {
        MAX_IMAGE
    }
}

fn ext(name: &str) -> Option<String> {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, e)| e.to_ascii_lowercase())
}

/// `rel` (resolved: `real`) of the agent's folder, sent whole up to `max`
/// bytes.
pub(super) fn read(exec: &dyn Exec, rel: &str, real: &str, max: u64) -> Res<FileView> {
    let view = |size, kind, text| FileView {
        path: rel.to_string(),
        size,
        kind,
        text,
        lang: lang(rel).map(String::from),
        lossy: false,
    };
    let size_of = || -> Res<u64> {
        match exec.stat(real)? {
            Some(s) if s.kind == FileKind::Dir => Err(format!("{rel}: is a folder")),
            Some(s) => Ok(s.len),
            None => Err(format!("{rel}: not found")),
        }
    };
    if is_svg(rel) {
        // Drawn, and its text sent too while it is small enough to edit.
        let size = size_of()?;
        if size > MAX_IMAGE {
            return Ok(view(size, ContentKind::Binary, None));
        }
        if size > MAX_TEXT {
            return Ok(view(size, ContentKind::Image, None));
        }
        let bytes = exec.read_file(real, MAX_TEXT + 1)?.ok_or_else(|| format!("{rel}: not found"))?;
        let text = (!looks_binary(&bytes) && bytes.len() as u64 <= MAX_TEXT).then(|| String::from_utf8_lossy(&bytes).into_owned());
        let mut v = view(bytes.len() as u64, ContentKind::Image, text);
        v.lossy = v.text.is_some() && std::str::from_utf8(&bytes).is_err();
        return Ok(v);
    }
    if is_image(rel) {
        let size = size_of()?;
        let kind = if size <= MAX_IMAGE { ContentKind::Image } else { ContentKind::Binary };
        return Ok(view(size, kind, None));
    }
    if binary_by_name(rel) {
        return Ok(view(size_of()?, ContentKind::Binary, None));
    }
    let bytes = match exec.read_file(real, max + 1) {
        Ok(Some(b)) => b,
        Ok(None) => return Err(format!("{rel}: not found")),
        Err(e) => {
            size_of()?; // a folder says so
            return Err(e.into());
        }
    };
    if bytes.len() as u64 > max {
        return Ok(view(size_of()?, ContentKind::TooLarge, None));
    }
    let size = bytes.len() as u64;
    if looks_binary(&bytes) {
        return Ok(view(size, ContentKind::Binary, None));
    }
    let mut v = view(size, ContentKind::Text, Some(String::from_utf8_lossy(&bytes).into_owned()));
    v.lossy = std::str::from_utf8(&bytes).is_err();
    Ok(v)
}

/// Its name says it is binary (its contents are never read).
pub fn binary_by_name(rel: &str) -> bool {
    ext(rel).is_some_and(|e| BINARY_EXT.contains(&e.as_str()))
}

/// A NUL near the start (git's rule).
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(SNIFF)].contains(&0)
}

/// A file's bytes (an image or a PDF for the viewer), up to `max`.
pub(super) fn read_bytes(exec: &dyn Exec, rel: &str, real: &str, max: u64) -> Res<Vec<u8>> {
    match exec.read_file(real, max + 1) {
        Ok(Some(b)) if b.len() as u64 > max => Err(format!("{rel}: larger than {} MiB", max / (1024 * 1024))),
        Ok(Some(b)) => Ok(b),
        Ok(None) => Err(format!("{rel}: not found")),
        Err(e) => Err(e.into()),
    }
}

/// A language id for the viewer's highlighter, from the file name.
pub fn lang(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let by_name = match name {
        "Dockerfile" | "Containerfile" => Some("dockerfile"),
        "Makefile" | "makefile" | "GNUmakefile" => Some("makefile"),
        "CMakeLists.txt" => Some("cmake"),
        "Cargo.lock" | "Pipfile" | "poetry.lock" | "uv.lock" => Some("toml"),
        "Gemfile" | "Rakefile" | "Podfile" | "Vagrantfile" => Some("ruby"),
        "go.mod" | "go.sum" => Some("go"),
        ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" | ".zprofile" => Some("shell"),
        ".gitignore" | ".dockerignore" | ".gitattributes" | ".npmignore" => Some("ignore"),
        ".env" => Some("properties"),
        _ if name.starts_with("Dockerfile.") => Some("dockerfile"),
        _ if name.starts_with(".env.") => Some("properties"),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    Some(match ext(name)?.as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "json" | "jsonc" | "json5" => "json",
        "md" | "markdown" | "mdx" => "markdown",
        "py" | "pyi" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "ino" => "cpp",
        "m" | "mm" => "objectivec",
        "cs" => "csharp",
        "fs" | "fsx" => "fsharp",
        "css" => "css",
        "scss" | "sass" => "scss",
        "less" => "less",
        "html" | "htm" => "html",
        "xml" | "svg" | "plist" | "xsd" | "xsl" => "xml",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "ini" | "cfg" | "conf" | "properties" => "properties",
        "sh" | "bash" | "zsh" | "fish" => "shell",
        "ps1" | "psm1" => "powershell",
        "bat" | "cmd" => "bat",
        "sql" => "sql",
        "rb" => "ruby",
        "php" => "php",
        "lua" => "lua",
        "pl" | "pm" => "perl",
        "r" => "r",
        "scala" | "sc" => "scala",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "clj" | "cljs" | "edn" => "clojure",
        "dart" => "dart",
        "zig" => "zig",
        "nix" => "nix",
        "proto" => "protobuf",
        "graphql" | "gql" => "graphql",
        "vue" => "vue",
        "svelte" => "svelte",
        "astro" => "astro",
        "diff" | "patch" => "diff",
        "tf" | "hcl" => "hcl",
        "ml" | "mli" => "ocaml",
        "jl" => "julia",
        "tex" => "latex",
        "dockerfile" => "dockerfile",
        "cmake" => "cmake",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeExec;

    fn read_text(x: &dyn Exec, rel: &str, real: &str) -> Res<FileView> {
        read(x, rel, real, MAX_TEXT)
    }

    #[test]
    fn text_binary_and_too_large() {
        let x = FakeExec::new();
        x.file("/r/a.rs", b"fn main() {}\n")
            .file("/r/nul.dat", b"ab\0cd")
            .file("/r/latin1.txt", b"caf\xe9\n")
            .file("/r/logo.png", b"\x89PNG not even read")
            .file("/r/big.log", &vec![b'x'; MAX_TEXT as usize + 1])
            .dir("/r/sub");
        let a = read_text(&*x, "a.rs", "/r/a.rs").unwrap();
        assert_eq!(
            (a.kind, a.text.as_deref(), a.size, a.lang.as_deref()),
            (ContentKind::Text, Some("fn main() {}\n"), 13, Some("rust"))
        );
        let n = read_text(&*x, "nul.dat", "/r/nul.dat").unwrap();
        assert_eq!((n.kind, n.text, n.size), (ContentKind::Binary, None, 5));
        let l = read_text(&*x, "latin1.txt", "/r/latin1.txt").unwrap();
        assert_eq!(
            l.text.as_deref(),
            Some("caf\u{fffd}\n"),
            "shown with a replacement character, as VS Code does"
        );
        let p = read_text(&*x, "logo.png", "/r/logo.png").unwrap();
        assert_eq!((p.kind, p.size, p.text), (ContentKind::Image, 18, None), "drawn by the viewer, not sent as text");
        assert_eq!(x.calls().len(), 0);
        let b = read_text(&*x, "big.log", "/r/big.log").unwrap();
        assert_eq!(
            (b.kind, b.text, b.size),
            (ContentKind::TooLarge, None, MAX_TEXT + 1)
        );
        let anyway = read(&*x, "big.log", "/r/big.log", MAX_LARGE).unwrap();
        assert_eq!(
            (anyway.kind, anyway.text.map(|t| t.len())),
            (ContentKind::Text, Some(MAX_TEXT as usize + 1)),
            "Load anyway reads up to 10 MiB"
        );
        assert_eq!(read_text(&*x, "sub", "/r/sub").unwrap_err(), "sub: is a folder");
        assert_eq!(read_bytes(&*x, "logo.png", "/r/logo.png", MAX_IMAGE).unwrap(), b"\x89PNG not even read");
        assert!(read_bytes(&*x, "logo.png", "/r/logo.png", 4).unwrap_err().contains("larger"));
    }

    #[test]
    fn images_and_their_cap() {
        let x = FakeExec::new();
        x.file("/r/a.SVG", b"<svg/>")
            .file("/r/huge.webp", &vec![0u8; MAX_IMAGE as usize + 1])
            .file("/r/clip.mp4", b"x");
        assert_eq!(read_text(&*x, "a.SVG", "/r/a.SVG").unwrap().kind, ContentKind::Image);
        assert_eq!(read_text(&*x, "huge.webp", "/r/huge.webp").unwrap().kind, ContentKind::Binary, "over the cap");
        assert_eq!(read_text(&*x, "clip.mp4", "/r/clip.mp4").unwrap().kind, ContentKind::Binary);
        assert!(is_image("x/y.JPeG") && !is_image("x/y.pdf") && !is_image("png"));
        assert_eq!(read_text(&*x, "gone", "/r/gone").unwrap_err(), "gone: not found");
    }

    #[test]
    fn an_svg_is_drawn_and_its_text_sent() {
        let x = FakeExec::new();
        x.file("/r/logo.svg", b"<svg xmlns='http://www.w3.org/2000/svg'/>\n")
            .file("/r/big.svg", &vec![b' '; MAX_TEXT as usize + 1])
            .file("/r/huge.svg", &vec![b' '; MAX_IMAGE as usize + 1])
            .file("/r/odd.svg", b"<svg>\xe9</svg>");
        let s = read_text(&*x, "logo.svg", "/r/logo.svg").unwrap();
        assert_eq!(
            (s.kind, s.text.as_deref(), s.lang.as_deref(), s.lossy),
            (ContentKind::Image, Some("<svg xmlns='http://www.w3.org/2000/svg'/>\n"), Some("xml"), false)
        );
        let b = read_text(&*x, "big.svg", "/r/big.svg").unwrap();
        assert_eq!((b.kind, b.text.is_none()), (ContentKind::Image, true), "drawn, too large for its source");
        assert_eq!(read_text(&*x, "huge.svg", "/r/huge.svg").unwrap().kind, ContentKind::Binary);
        assert!(read_text(&*x, "odd.svg", "/r/odd.svg").unwrap().lossy);
        assert!(is_svg("a/B.SVG") && !is_svg("a.png") && !is_svg("svg"));
    }

    #[test]
    fn pdfs_are_binary_and_their_bytes_capped_higher() {
        let x = FakeExec::new();
        x.file("/r/doc.pdf", b"%PDF-1.4 not read");
        let p = read_text(&*x, "doc.pdf", "/r/doc.pdf").unwrap();
        assert_eq!((p.kind, p.size, p.text), (ContentKind::Binary, 17, None));
        assert_eq!(x.calls().len(), 0, "never read as text");
        assert!(is_pdf("x/Report.PDF") && !is_pdf("pdf") && !is_pdf("a.pdf.txt"));
        assert_eq!((bytes_cap("a.pdf"), bytes_cap("a.png")), (MAX_PDF, MAX_IMAGE));
        assert_eq!(read_bytes(&*x, "doc.pdf", "/r/doc.pdf", bytes_cap("doc.pdf")).unwrap(), b"%PDF-1.4 not read");
    }

    #[test]
    fn language_hints() {
        assert_eq!(lang("src/App.tsx"), Some("tsx"));
        assert_eq!(lang("Dockerfile"), Some("dockerfile"));
        assert_eq!(lang("a/Cargo.lock"), Some("toml"));
        assert_eq!(lang("x/.env.local"), Some("properties"));
        assert_eq!(lang("README.MD"), Some("markdown"));
        assert_eq!(lang(".bashrc"), Some("shell"));
        assert_eq!(lang("LICENSE"), None);
        assert_eq!(lang("weird.zzz"), None);
    }
}
