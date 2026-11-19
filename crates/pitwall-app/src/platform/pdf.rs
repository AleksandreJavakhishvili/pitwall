//! PDFs for the file viewer, drawn by the system's PDF engine: macOS
//! (PDFKit, `pdf_macos.rs`). Elsewhere [`open`] answers
//! [`OpenError::Unsupported`] and the viewer says so
//! ([`super::can_view_pdf`]).
//!
//! A document lives on a thread of its own (PDFKit's objects stay on the
//! thread that made them); the viewer sends it jobs (draw a page at a
//! size, find a text) and waits for each answer on the background
//! executor, as for the explorer's other blocking calls, so the UI thread
//! never waits on the engine. The thread ends when the [`Pdf`] is
//! dropped.

use std::sync::{mpsc, Mutex};

/// Why a PDF can't be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// It needs a password to be opened.
    Locked,
    /// The engine can't read it (why, when known).
    Damaged(String),
    /// No PDF engine here (Windows, Linux).
    Unsupported,
}

/// A page drawn: `w` × `h` pixels, BGRA with premultiplied alpha (the
/// order GPUI draws), white where the page has nothing.
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

/// One place a search found the text: its page and its box on the page
/// (`[x, y, w, h]` in points from the page's top left, as drawn: cropped
/// and rotated).
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub page: usize,
    pub rect: [f32; 4],
}

/// At most this many hits are kept from one search.
pub const MAX_HITS: usize = 5000;

/// A job for the document's thread.
// (Built only where PDFs can be drawn: never on Windows or Linux.)
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) enum Job {
    Render { page: usize, w: u32, h: u32, reply: mpsc::Sender<Result<Bitmap, String>> },
    Find { query: String, reply: mpsc::Sender<Vec<Hit>> },
}

/// An open PDF: its pages' sizes, and its thread.
// (Made only where PDFs can be drawn: never on Windows or Linux.)
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct Pdf {
    jobs: Mutex<mpsc::Sender<Job>>,
    /// Each page's size in points (`w`, `h`), as drawn: its crop box,
    /// turned by its rotation.
    pages: Vec<(f32, f32)>,
}

impl Pdf {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn new(jobs: mpsc::Sender<Job>, pages: Vec<(f32, f32)>) -> Pdf {
        Pdf { jobs: Mutex::new(jobs), pages }
    }

    pub fn pages(&self) -> &[(f32, f32)] {
        &self.pages
    }

    fn send(&self, job: Job) {
        if let Ok(tx) = self.jobs.lock() {
            let _ = tx.send(job);
        }
    }

    /// Page `page` drawn at `w` × `h` pixels. Blocks until it is.
    pub fn render(&self, page: usize, w: u32, h: u32) -> Result<Bitmap, String> {
        let (reply, rx) = mpsc::channel();
        self.send(Job::Render { page, w, h, reply });
        rx.recv().map_err(|_| "the PDF engine stopped".to_string())?
    }

    /// Where `query` is in the document (ignoring case and accents), in
    /// reading order, at most [`MAX_HITS`]. Blocks until it is known.
    pub fn find(&self, query: &str) -> Vec<Hit> {
        let (reply, rx) = mpsc::channel();
        self.send(Job::Find { query: query.to_string(), reply });
        rx.recv().unwrap_or_default()
    }
}

/// Open a PDF from its bytes. Blocks while the engine reads it: call it
/// off the UI thread.
pub fn open(bytes: Vec<u8>) -> Result<Pdf, OpenError> {
    #[cfg(target_os = "macos")]
    {
        super::pdf_macos::open(bytes)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        Err(OpenError::Unsupported)
    }
}

/// A page's size as drawn: its box (`w`, `h`) turned by `rotation`
/// degrees (a multiple of 90, maybe negative or over 360).
pub fn turned(w: f32, h: f32, rotation: i64) -> (f32, f32) {
    if rotation.rem_euclid(180) == 90 {
        (h, w)
    } else {
        (w, h)
    }
}

/// A small PDF made by hand, for tests: one page per `(w, h, rotate,
/// text)`, the text at (72, h − 100) in 24 pt Helvetica, over a black
/// square at (0, 0)–(40, 40) (the page's bottom left before rotation).
#[cfg(test)]
pub fn sample(pages: &[(u32, u32, i32, &str)]) -> Vec<u8> {
    let mut objs: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()));
    objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for (i, (w, h, rot, text)) in pages.iter().enumerate() {
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Rotate {rot} /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * i
        ));
        let stream = format!("0 0 0 rg 0 0 40 40 re f BT /F1 24 Tf 72 {} Td ({text}) Tj ET", h - 100);
        objs.push(format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()));
    }
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
    for o in offsets {
        out.push_str(&format!("{o:010} 00000 n \n"));
    }
    out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1));
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_page_swaps_its_sides() {
        assert_eq!(turned(612., 792., 0), (612., 792.));
        assert_eq!(turned(612., 792., 90), (792., 612.));
        assert_eq!(turned(612., 792., 180), (612., 792.));
        assert_eq!(turned(612., 792., -90), (792., 612.));
        assert_eq!(turned(612., 792., 450), (792., 612.));
    }

    #[test]
    fn opening_says_why_it_cant() {
        let made = open(sample(&[(200, 300, 0, "Hi")]));
        if super::super::can_view_pdf() {
            assert_eq!(made.map(|p| p.pages().to_vec()).ok(), Some(vec![(200., 300.)]));
            assert!(matches!(open(b"%PDF-1.4 nothing else".to_vec()), Err(OpenError::Damaged(_))));
            assert!(matches!(open(Vec::new()), Err(OpenError::Damaged(_))));
            // Made by Ghostscript with a user password ("secret").
            assert_eq!(open(include_bytes!("testdata/locked.pdf").to_vec()).err(), Some(OpenError::Locked));
        } else {
            assert_eq!(made.err(), Some(OpenError::Unsupported));
        }
    }
}
