//! Real PNG files for tests and the seed extensions' art: an RGBA image
//! from a function of each pixel, encoded with zlib (flate2) and CRC-32,
//! the same bytes every time.

use std::io::Write as _;

/// A `w`×`h` RGBA PNG whose pixel at (x, y) is `pixel(x, y)`.
pub fn png(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 4 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&raw).expect("in memory");
    let data = z.finish().expect("in memory");
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &data);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A plain `w`×`h` PNG of one colour.
pub fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    png(w, h, |_, _| rgba)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// Drawing for the seed tiles' icons (tiles/c4, tiles/notes): shapes as
/// signed distances, 4×4 samples a pixel, on a rounded square with a
/// vertical gradient. No third-party art; the same bytes every time.
pub mod art {
    /// A shape's signed distance at (x, y): below 0 inside.
    pub type Shape = Box<dyn Fn(f32, f32) -> f32>;

    /// A rounded rectangle centred at (cx, cy), half sizes (hw, hh),
    /// corner radius r.
    pub fn rrect(cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> Shape {
        Box::new(move |x, y| {
            let qx = (x - cx).abs() - hw + r;
            let qy = (y - cy).abs() - hh + r;
            let out = (qx.max(0.).powi(2) + qy.max(0.).powi(2)).sqrt();
            out + qx.max(qy).min(0.) - r
        })
    }

    /// A line from (ax, ay) to (bx, by), `w` wide with round ends.
    pub fn line(ax: f32, ay: f32, bx: f32, by: f32, w: f32) -> Shape {
        Box::new(move |x, y| {
            let (px, py, dx, dy) = (x - ax, y - ay, bx - ax, by - ay);
            let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
            ((px - dx * t).powi(2) + (py - dy * t).powi(2)).sqrt() - w / 2.
        })
    }

    /// `a` without `b`.
    pub fn minus(a: Shape, b: Shape) -> Shape {
        Box::new(move |x, y| a(x, y).max(-b(x, y)))
    }

    /// A layer: a shape and its colour (RGBA, 0–1).
    pub struct Layer {
        pub shape: Shape,
        pub color: [f32; 4],
    }

    /// A `size`² icon: a rounded square (corner 22%) filled from `top` to
    /// `bottom`, then `layers` over it, on a 0–1 square.
    pub fn icon(size: u32, top: [f32; 3], bottom: [f32; 3], layers: &[Layer]) -> Vec<u8> {
        let tile = rrect(0.5, 0.5, 0.5, 0.5, 0.22);
        let n = 4;
        super::png(size, size, |px, py| {
            let mut acc = [0f32; 4];
            for sy in 0..n {
                for sx in 0..n {
                    let x = (px as f32 + (sx as f32 + 0.5) / n as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / n as f32) / size as f32;
                    if tile(x, y) > 0. {
                        continue;
                    }
                    let mut c: [f32; 3] = std::array::from_fn(|i| top[i] + (bottom[i] - top[i]) * y);
                    for l in layers {
                        if (l.shape)(x, y) <= 0. {
                            let a = l.color[3];
                            c = std::array::from_fn(|i| c[i] * (1. - a) + l.color[i] * a);
                        }
                    }
                    for (s, v) in acc.iter_mut().zip(c) {
                        *s += v;
                    }
                    acc[3] += 1.;
                }
            }
            let s = (n * n) as f32;
            if acc[3] == 0. {
                return [0, 0, 0, 0];
            }
            let a = acc[3] / s;
            [(acc[0] / acc[3] * 255.).round() as u8, (acc[1] / acc[3] * 255.).round() as u8, (acc[2] / acc[3] * 255.).round() as u8, (a * 255.).round() as u8]
        })
    }

    /// C4 diagrams' icon: three boxes joined, on indigo.
    pub fn c4(size: u32) -> Vec<u8> {
        let w = [1., 1., 1., 1.];
        let soft = [1., 1., 1., 0.78];
        icon(
            size,
            [0.36, 0.42, 0.96],
            [0.22, 0.25, 0.74],
            &[
                Layer { shape: line(0.5, 0.36, 0.3, 0.62, 0.045), color: soft },
                Layer { shape: line(0.5, 0.36, 0.7, 0.62, 0.045), color: soft },
                Layer { shape: rrect(0.5, 0.31, 0.15, 0.09, 0.035), color: w },
                Layer { shape: rrect(0.3, 0.68, 0.13, 0.08, 0.03), color: w },
                Layer { shape: rrect(0.7, 0.68, 0.13, 0.08, 0.03), color: w },
            ],
        )
    }

    /// Notes' icon: a page with lines and a folded corner, on amber.
    pub fn notes(size: u32) -> Vec<u8> {
        let page = minus(rrect(0.5, 0.5, 0.25, 0.31, 0.045), line(0.75, 0.19, 0.75, 0.19, 0.24));
        let ink = [0.86, 0.55, 0.1, 1.];
        icon(
            size,
            [1.0, 0.78, 0.28],
            [0.96, 0.58, 0.12],
            &[
                Layer { shape: page, color: [1., 0.99, 0.96, 1.] },
                Layer { shape: rrect(0.69, 0.25, 0.06, 0.06, 0.02), color: [0.99, 0.88, 0.62, 1.] },
                Layer { shape: line(0.36, 0.42, 0.58, 0.42, 0.04), color: ink },
                Layer { shape: line(0.36, 0.53, 0.64, 0.53, 0.04), color: ink },
                Layer { shape: line(0.36, 0.64, 0.56, 0.64, 0.04), color: ink },
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    /// The seed tiles' icons, written into their folders: run by hand
    /// after changing their art (`PITWALL_SEED_ART=1 cargo test -p
    /// pitwall-core seed_icons -- --ignored`).
    #[test]
    #[ignore]
    fn seed_icons() {
        if std::env::var_os("PITWALL_SEED_ART").is_none() {
            return;
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tiles");
        for (dir, art) in [("c4", super::art::c4 as fn(u32) -> Vec<u8>), ("notes", super::art::notes)] {
            let to = root.join(dir).join("assets");
            std::fs::create_dir_all(&to).unwrap();
            let bytes = art(256);
            crate::extensions::media::check_icon("icon.png", &bytes).unwrap();
            std::fs::write(to.join("icon.png"), bytes).unwrap();
        }
    }

    #[test]
    fn the_seed_icons_are_the_same_every_time() {
        let a = super::art::notes(64);
        assert_eq!(a, super::art::notes(64));
        assert_eq!(crate::extensions::media::check_icon("i.png", &a), Ok(64));
    }

    #[test]
    fn a_png_reads_as_one() {
        let p = super::solid(64, 64, [10, 20, 30, 255]);
        assert_eq!(crate::extensions::media::check_icon("i.png", &p), Ok(64));
        assert_eq!(super::crc32(b"IEND"), 0xAE42_6082);
    }
}
