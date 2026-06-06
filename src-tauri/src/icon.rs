//! Dynamically rendered tray icon: two ring gauges side by side —
//! left = 5-hour window (current, coral), right = weekly (blue) — each filled
//! by its utilization with the number in the center. Redrawn every poll.

use tauri::image::Image;

// 5x7 bitmap font for digits 0-9 (bit4 = leftmost column).
const DIGITS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

const CORAL: (u8, u8, u8) = (217, 119, 87); // 5-hour (current)
const BLUE: (u8, u8, u8) = (91, 155, 213); // weekly
const TRACK: (u8, u8, u8) = (70, 70, 88);
const DISC: (u8, u8, u8) = (28, 28, 40);

fn escalate(util: f64, base: (u8, u8, u8), warn: f64, crit: f64) -> (u8, u8, u8) {
    if util >= crit {
        (230, 75, 58)
    } else if util >= warn {
        (230, 177, 58)
    } else {
        base
    }
}

struct Canvas {
    w: i32,
    h: i32,
    buf: Vec<u8>,
}

impl Canvas {
    fn new(w: i32, h: i32) -> Self {
        Canvas {
            w,
            h,
            buf: vec![0u8; (w * h * 4) as usize],
        }
    }

    fn put(&mut self, x: i32, y: i32, c: (u8, u8, u8), a: u8) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        self.buf[i] = c.0;
        self.buf[i + 1] = c.1;
        self.buf[i + 2] = c.2;
        self.buf[i + 3] = a;
    }

    /// Draw a ring gauge centered at (cx, cy): dark disc, track, and a fill arc
    /// from the top going clockwise proportional to `util` (0..=100).
    fn ring(&mut self, cx: f32, cy: f32, r_out: f32, r_in: f32, util: f64, fill: (u8, u8, u8)) {
        let fill_deg = (util.clamp(0.0, 100.0) / 100.0 * 360.0) as f32;
        let x0 = (cx - r_out).floor() as i32;
        let x1 = (cx + r_out).ceil() as i32;
        let y0 = (cy - r_out).floor() as i32;
        let y1 = (cy + r_out).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= r_in {
                    self.put(x, y, DISC, 235);
                } else if dist <= r_out {
                    let mut deg = dx.atan2(-dy).to_degrees();
                    if deg < 0.0 {
                        deg += 360.0;
                    }
                    if deg <= fill_deg {
                        self.put(x, y, fill, 255);
                    } else {
                        self.put(x, y, TRACK, 255);
                    }
                }
            }
        }
    }

    /// Draw a number centered at (cx, cy).
    fn number(&mut self, cx: i32, cy: i32, n: i32, scale: i32, color: (u8, u8, u8)) {
        let label = n.to_string();
        let glyph_w = 5 * scale;
        let spacing = scale;
        let text_w = label.len() as i32 * glyph_w + (label.len() as i32 - 1) * spacing;
        let text_h = 7 * scale;
        let mut ox = cx - text_w / 2;
        let oy = cy - text_h / 2;
        for ch in label.chars() {
            if let Some(d) = ch.to_digit(10) {
                let pattern = &DIGITS[d as usize];
                for (row, bits) in pattern.iter().enumerate() {
                    for col in 0..5 {
                        if bits & (1 << (4 - col)) != 0 {
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    self.put(ox + col * scale + sx, oy + row as i32 * scale + sy, color, 255);
                                }
                            }
                        }
                    }
                }
            }
            ox += glyph_w + spacing;
        }
    }

    fn into_image(self) -> Image<'static> {
        Image::new_owned(self.buf, self.w as u32, self.h as u32)
    }
}

/// Two side-by-side ring gauges: left = 5h (current), right = weekly.
pub fn gauge_dual(five: Option<f64>, seven: Option<f64>, warn: f64, crit: f64) -> Image<'static> {
    const W: i32 = 76;
    const H: i32 = 38;
    let r_out = 17.0;
    let r_in = 11.5;
    let mut c = Canvas::new(W, H);

    // left ring — 5-hour window
    let fv = five.unwrap_or(0.0);
    c.ring(18.0, 19.0, r_out, r_in, fv, escalate(fv, CORAL, warn, crit));
    if five.is_some() {
        let scale = if fv.round() >= 100.0 { 1 } else { 2 };
        c.number(18, 19, fv.round() as i32, scale, (245, 245, 250));
    }

    // right ring — weekly
    let sv = seven.unwrap_or(0.0);
    c.ring(58.0, 19.0, r_out, r_in, sv, escalate(sv, BLUE, warn, crit));
    if seven.is_some() {
        let scale = if sv.round() >= 100.0 { 1 } else { 2 };
        c.number(58, 19, sv.round() as i32, scale, (245, 245, 250));
    }

    c.into_image()
}
