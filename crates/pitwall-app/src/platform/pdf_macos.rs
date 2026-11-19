//! macOS: PDFs through PDFKit (`PDFDocument`, `PDFPage`), each document on
//! a thread of its own, pages drawn into CoreGraphics bitmaps the viewer
//! shows as images (never a native `PDFView`: that would sit above
//! everything GPUI draws, menus and drag previews included).

use std::sync::mpsc;

use objc2::rc::{autoreleasepool, Retained};
use objc2::AnyThread as _;
use objc2_core_foundation::{CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo};
use objc2_foundation::{NSData, NSString, NSStringCompareOptions};
use objc2_pdf_kit::{PDFDisplayBox, PDFDocument, PDFPage};

use super::pdf::{turned, Bitmap, Hit, Job, OpenError, Pdf, MAX_HITS};

/// The box pages are shown by: what a reader shows.
const BOX: PDFDisplayBox = PDFDisplayBox::CropBox;

/// A bitmap is never larger than this many pixels a side (GPUI's textures
/// stop at 16 384; this keeps a page's memory in reason).
pub const MAX_SIDE: u32 = 8192;

pub fn open(bytes: Vec<u8>) -> Result<Pdf, OpenError> {
    let (jobs, rx) = mpsc::channel::<Job>();
    let (opened, wait) = mpsc::channel();
    std::thread::Builder::new()
        .name("pitwall-pdf".into())
        .spawn(move || {
            let doc = autoreleasepool(|_| load(&bytes));
            drop(bytes);
            let doc = match doc {
                Ok((doc, pages)) => {
                    let _ = opened.send(Ok(pages));
                    doc
                }
                Err(e) => {
                    let _ = opened.send(Err(e));
                    return;
                }
            };
            for job in rx {
                autoreleasepool(|_| run(&doc, job));
            }
        })
        .map_err(|e| OpenError::Damaged(format!("couldn't start reading it: {e}")))?;
    let pages = wait.recv().map_err(|_| OpenError::Damaged("the PDF engine stopped".into()))??;
    Ok(Pdf::new(jobs, pages))
}

/// An opened document and its pages' sizes.
type Loaded = (Retained<PDFDocument>, Vec<(f32, f32)>);

fn load(bytes: &[u8]) -> Result<Loaded, OpenError> {
    if bytes.is_empty() {
        return Err(OpenError::Damaged("the file is empty".into()));
    }
    let data = NSData::with_bytes(bytes);
    // SAFETY: a fresh PDFDocument from data that outlives the call (it
    // keeps its own reference).
    let doc = unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) }
        .ok_or_else(|| OpenError::Damaged("it isn't a PDF the system can read".into()))?;
    // SAFETY: plain PDFDocument getters on the thread that made it.
    unsafe {
        // (A document with an empty password opens by itself.)
        if doc.isLocked() && !doc.unlockWithPassword(&NSString::from_str("")) {
            return Err(OpenError::Locked);
        }
        let n = doc.pageCount();
        if n == 0 {
            return Err(OpenError::Damaged("it has no pages".into()));
        }
        let mut pages = Vec::with_capacity(n);
        for i in 0..n {
            let size = doc.pageAtIndex(i).map(|p| shown_size(&p)).unwrap_or((612., 792.));
            pages.push(size);
        }
        Ok((doc, pages))
    }
}

/// A page's size as shown (points).
fn shown_size(page: &PDFPage) -> (f32, f32) {
    // SAFETY: PDFPage getters.
    let (b, r) = unsafe { (page.boundsForBox(BOX), page.rotation()) };
    turned(b.size.width.max(1.) as f32, b.size.height.max(1.) as f32, r as i64)
}

fn run(doc: &PDFDocument, job: Job) {
    match job {
        Job::Render { page, w, h, reply } => {
            let _ = reply.send(render(doc, page, w, h));
        }
        Job::Find { query, reply } => {
            let _ = reply.send(find(doc, &query));
        }
    }
}

fn render(doc: &PDFDocument, index: usize, w: u32, h: u32) -> Result<Bitmap, String> {
    // SAFETY: PDFDocument getters.
    let page = unsafe { doc.pageAtIndex(index) }.ok_or_else(|| format!("no page {}", index + 1))?;
    let (w, h) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
    let (pw, ph) = shown_size(&page);
    let row = w as usize * 4;
    let mut bgra = vec![0u8; row * h as usize];
    let space = CGColorSpace::new_device_rgb().ok_or("no colour space")?;
    let info = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;
    // SAFETY: `bgra` is `row` × `h` bytes and outlives the context, which
    // is dropped below before it is read.
    let ctx = unsafe { CGBitmapContextCreate(bgra.as_mut_ptr().cast(), w as usize, h as usize, 8, row, Some(&space), info) }
        .ok_or("couldn't make a bitmap")?;
    let full = CGRect::new(CGPoint::new(0., 0.), CGSize::new(w as f64, h as f64));
    // Paper: white under the page's own drawing.
    CGContext::set_rgb_fill_color(Some(&ctx), 1., 1., 1., 1.);
    CGContext::fill_rect(Some(&ctx), full);
    CGContext::scale_ctm(Some(&ctx), w as f64 / pw as f64, h as f64 / ph as f64);
    // SAFETY: draws the page (its rotation and box offset applied by
    // PDFKit) into our context, on the document's thread.
    unsafe { page.drawWithBox_toContext(BOX, &ctx) };
    drop(ctx);
    Ok(Bitmap { w, h, bgra })
}

fn find(doc: &PDFDocument, query: &str) -> Vec<Hit> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let opts = NSStringCompareOptions::CaseInsensitiveSearch | NSStringCompareOptions::DiacriticInsensitiveSearch;
    let mut hits = Vec::new();
    // SAFETY: PDFDocument / PDFSelection getters on the document's thread.
    unsafe {
        let found = doc.findString_withOptions(&NSString::from_str(query), opts);
        for sel in found.iter() {
            for page in sel.pages().iter() {
                let index = doc.indexForPage(&page);
                let bounds = sel.boundsForPage(&page);
                let shown = shown_size(&page);
                let t = page.transformForBox(BOX);
                hits.push(Hit { page: index, rect: to_shown(bounds, t, shown.1) });
                if hits.len() >= MAX_HITS {
                    return hits;
                }
            }
        }
    }
    hits
}

/// `r` in page space, as a box from the top left of the page as shown
/// (`t`: page space to the shown box, its bottom left at 0; `h`: its
/// height).
fn to_shown(r: CGRect, t: CGAffineTransform, h: f32) -> [f32; 4] {
    let corners = [
        (r.origin.x, r.origin.y),
        (r.origin.x + r.size.width, r.origin.y),
        (r.origin.x, r.origin.y + r.size.height),
        (r.origin.x + r.size.width, r.origin.y + r.size.height),
    ];
    let pts = corners.map(|(x, y)| (t.a * x + t.c * y + t.tx, t.b * x + t.d * y + t.ty));
    let x0 = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let x1 = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let y0 = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let y1 = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    [x0 as f32, h - y1 as f32, (x1 - x0) as f32, (y1 - y0) as f32]
}

#[cfg(test)]
mod tests {
    use super::super::pdf::{open, sample};
    use super::*;

    /// The pixel at (x, y) from the top left: (b, g, r).
    fn px(b: &Bitmap, x: u32, y: u32) -> (u8, u8, u8) {
        let i = (y * b.w + x) as usize * 4;
        (b.bgra[i], b.bgra[i + 1], b.bgra[i + 2])
    }

    #[test]
    fn pages_are_drawn_upright_on_white() {
        let pdf = open(sample(&[(200, 300, 0, "Hello"), (200, 300, 90, "Turned")])).unwrap();
        assert_eq!(pdf.pages(), &[(200., 300.), (300., 200.)]);
        // Twice the size: the black square is the bottom left 80 px.
        let b = pdf.render(0, 400, 600).unwrap();
        assert_eq!((b.w, b.h, b.bgra.len()), (400, 600, 400 * 600 * 4));
        assert_eq!(px(&b, 10, 590), (0, 0, 0), "the square, bottom left");
        assert_eq!(px(&b, 390, 10), (255, 255, 255), "paper, top right");
        // Turned a quarter clockwise: the bottom left goes to the top left.
        let t = pdf.render(1, 300, 200).unwrap();
        assert_eq!(px(&t, 10, 10), (0, 0, 0), "the square, top left once turned");
        assert_eq!(px(&t, 290, 190), (255, 255, 255));
        assert!(pdf.render(9, 10, 10).is_err());
    }

    #[test]
    fn finding_text_gives_its_box_as_shown() {
        let pdf = open(sample(&[(200, 300, 0, "Alpha beta"), (200, 300, 0, "BETA"), (200, 300, 90, "beta")])).unwrap();
        let hits = pdf.find("beta");
        assert_eq!(hits.iter().map(|h| h.page).collect::<Vec<_>>(), vec![0, 1, 2], "{hits:?}");
        // The text's baseline is 100 pt below the top, 24 pt high: its box
        // ends near y = 100 from the top.
        let [x, y, w, h] = hits[0].rect;
        assert!(x > 72. && x < 200. && w > 10. && h > 10., "{:?}", hits[0]);
        assert!(y > 60. && y < 100. && y + h > 90. && y + h < 110., "{:?}", hits[0]);
        // Turned: the text runs down the page's right part (300 × 200).
        let [x, y, w, h] = hits[2].rect;
        assert!(h > w, "runs downward: {:?}", hits[2]);
        assert!(x > 150. && x < 300. && y > 50., "{:?}", hits[2]);
        assert!(pdf.find("  ").is_empty());
        assert!(pdf.find("gamma").is_empty());
    }
}
