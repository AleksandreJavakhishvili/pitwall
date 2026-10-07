//! The "N agents need you" badge as a small RGBA image (Windows taskbar
//! overlay icon; macOS draws its own Dock badge from a number). Pure, so it
//! is tested on every OS.

/// Side of the square image, in pixels (shown at 16 px, sharp at 200 %).
pub const SIZE: u32 = 32;

const RED: [u8; 4] = [0xE5, 0x48, 0x4D, 0xFF];
const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// 3×5 glyphs, one row per entry, high bit first.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b001, 0b001],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        _ => [0b000, 0b010, 0b111, 0b010, 0b000], // '+'
    }
}

/// What the badge says: the count, or "9+" past 99.
pub fn label(count: usize) -> String {
    if count > 99 {
        "9+".into()
    } else {
        count.to_string()
    }
}

/// A red disc with the count in white: `SIZE`×`SIZE` RGBA bytes.
pub fn rgba(count: usize) -> Vec<u8> {
    let n = SIZE as i32;
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    let mut put = |x: i32, y: i32, c: [u8; 4]| {
        if (0..n).contains(&x) && (0..n).contains(&y) {
            let i = ((y * n + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&c);
        }
    };
    let r = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 + 0.5 - r, y as f32 + 0.5 - r);
            if dx * dx + dy * dy <= (r - 0.5) * (r - 0.5) {
                put(x, y, RED);
            }
        }
    }
    let text = label(count);
    let scale: i32 = if text.len() == 1 { 4 } else { 3 };
    let (w, h) = (text.len() as i32 * 4 * scale - scale, 5 * scale);
    let (x0, y0) = ((n - w) / 2, (n - h) / 2);
    for (i, c) in text.chars().enumerate() {
        let g = glyph(c);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            put(x0 + (i as i32 * 4 + col) * scale + sx, y0 + row as i32 * scale + sy, WHITE);
                        }
                    }
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(px: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        px[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn a_red_disc_with_white_digits() {
        assert_eq!((label(3), label(42), label(120)), ("3".to_string(), "42".to_string(), "9+".to_string()));
        let px = rgba(1);
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(at(&px, 0, 0)[3], 0, "corners are transparent");
        assert_eq!(at(&px, 3, SIZE / 2), RED);
        // "1": its stem is the middle column of the glyph.
        assert_eq!(at(&px, SIZE / 2, SIZE / 2), WHITE);
        assert!(rgba(88).chunks(4).filter(|p| *p == WHITE).count() > rgba(1).chunks(4).filter(|p| *p == WHITE).count());
    }
}
