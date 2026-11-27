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

#[cfg(test)]
mod tests {
    #[test]
    fn a_png_reads_as_one() {
        let p = super::solid(64, 64, [10, 20, 30, 255]);
        assert_eq!(crate::extensions::media::check_icon("i.png", &p), Ok(64));
        assert_eq!(super::crc32(b"IEND"), 0xAE42_6082);
    }
}
