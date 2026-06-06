//! Dynamically rendered tray icon: a Claude-orange ring gauge that fills with
//! usage %, with the number drawn in the center. Updated on every poll so the
//! current quota is visible at a glance in the system tray.

use tauri::image::Image;

const SIZE: i32 = 36;
const R_OUTER: f32 = 17.0;
const R_INNER: f32 = 12.5;

// 5x7 bitmap font for digits 0-9 (bit4 = leftmost column).
const DIGITS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110], // 0
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110], // 1
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111], // 2
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110], // 3
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010], // 4
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110], // 5
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110], // 6
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000], // 7
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110], // 8
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100], // 9
];

fn level_color(level: &str) -> (u8, u8, u8) {
    match level {
        "crit" => (230, 75, 58),
        "warn" => (230, 177, 58),
        _ => (217, 119, 87), // Claude coral
    }
}

/// Build the tray icon for a given utilization (0..=100) and status level.
pub fn gauge(util: f64, level: &str) -> Image<'static> {
    let w = SIZE as usize;
    let mut buf = vec![0u8; w * w * 4];

    let (fr, fg, fb) = level_color(level);
    let (tr, tg, tb) = (70u8, 70u8, 88u8); // track
    let (dr, dg, db) = (28u8, 28u8, 40u8); // center disc
    let cx = SIZE as f32 / 2.0 - 0.5;
    let cy = SIZE as f32 / 2.0 - 0.5;
    let util = util.clamp(0.0, 100.0);
    let fill_deg = (util / 100.0 * 360.0) as f32;

    let mut put = |x: i32, y: i32, r: u8, g: u8, b: u8, a: u8| {
        if x < 0 || y < 0 || x >= SIZE || y >= SIZE {
            return;
        }
        let i = (y as usize * w + x as usize) * 4;
        buf[i] = r;
        buf[i + 1] = g;
        buf[i + 2] = b;
        buf[i + 3] = a;
    };

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= R_INNER {
                put(x, y, dr, dg, db, 235); // center disc (number background)
            } else if dist <= R_OUTER {
                // angle from top, clockwise, 0..360
                let mut deg = dx.atan2(-dy).to_degrees();
                if deg < 0.0 {
                    deg += 360.0;
                }
                if deg <= fill_deg {
                    put(x, y, fr, fg, fb, 255);
                } else {
                    put(x, y, tr, tg, tb, 255);
                }
            }
        }
    }

    // Draw the number centered on the disc.
    let label = format!("{}", util.round() as i32);
    let scale: i32 = if label.len() <= 2 { 2 } else { 1 };
    let glyph_w = 5 * scale;
    let spacing = scale;
    let text_w = label.len() as i32 * glyph_w + (label.len() as i32 - 1) * spacing;
    let text_h = 7 * scale;
    let mut ox = (SIZE - text_w) / 2;
    let oy = (SIZE - text_h) / 2;

    for ch in label.chars() {
        if let Some(d) = ch.to_digit(10) {
            let pattern = &DIGITS[d as usize];
            for (row, bits) in pattern.iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) != 0 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                put(
                                    ox + col * scale + sx,
                                    oy + row as i32 * scale + sy,
                                    245,
                                    245,
                                    250,
                                    255,
                                );
                            }
                        }
                    }
                }
            }
        }
        ox += glyph_w + spacing;
    }

    Image::new_owned(buf, SIZE as u32, SIZE as u32)
}
