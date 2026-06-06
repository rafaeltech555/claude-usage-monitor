//! Dynamically rendered tray icon: two ring gauges side by side —
//! left = 5-hour window (current, coral), right = weekly (blue) — each filled
//! by its utilization with the number in the center.
//!
//! Two optional states:
//! - flames overlaid on a ring whose usage just rose (animated by frame);
//! - a frozen/iced look when the quota data is stale (token expired — i.e.
//!   Claude Code hasn't been opened in a while).

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

// Frozen palette
const ICE_FILL: (u8, u8, u8) = (150, 190, 220);
const ICE_TRACK: (u8, u8, u8) = (52, 66, 84);
const ICE_DISC: (u8, u8, u8) = (34, 46, 62);
const FROST: (u8, u8, u8) = (210, 235, 250);

const W: i32 = 76;
const H: i32 = 38;
const R_OUT: f32 = 17.0;
const R_IN: f32 = 11.5;
const CY: f32 = 19.0;
const LEFT_CX: f32 = 18.0;
const RIGHT_CX: f32 = 58.0;

fn escalate(util: f64, base: (u8, u8, u8), warn: f64, crit: f64) -> (u8, u8, u8) {
    if util >= crit {
        (230, 75, 58)
    } else if util >= warn {
        (230, 177, 58)
    } else {
        base
    }
}

fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

fn flame_color(t: f32) -> (u8, u8, u8) {
    if t < 0.4 {
        lerp((255, 245, 200), (255, 175, 60), t / 0.4)
    } else if t < 0.75 {
        lerp((255, 175, 60), (240, 90, 30), (t - 0.4) / 0.35)
    } else {
        lerp((240, 90, 30), (200, 40, 25), (t - 0.75) / 0.25)
    }
}

struct Canvas {
    buf: Vec<u8>,
    frozen: bool,
}

impl Canvas {
    fn new(frozen: bool) -> Self {
        Canvas {
            buf: vec![0u8; (W * H * 4) as usize],
            frozen,
        }
    }

    fn track(&self) -> (u8, u8, u8) {
        if self.frozen { ICE_TRACK } else { TRACK }
    }
    fn disc(&self) -> (u8, u8, u8) {
        if self.frozen { ICE_DISC } else { DISC }
    }

    fn blend(&mut self, x: i32, y: i32, c: (u8, u8, u8), a: u8) {
        if x < 0 || y < 0 || x >= W || y >= H {
            return;
        }
        let i = ((y * W + x) * 4) as usize;
        let af = a as f32 / 255.0;
        for (k, ch) in [c.0, c.1, c.2].iter().enumerate() {
            self.buf[i + k] = (*ch as f32 * af + self.buf[i + k] as f32 * (1.0 - af)) as u8;
        }
        self.buf[i + 3] = self.buf[i + 3].max(a);
    }

    fn ring(&mut self, cx: f32, util: f64, fill: (u8, u8, u8)) {
        let fill_deg = (util.clamp(0.0, 100.0) / 100.0 * 360.0) as f32;
        let (track, disc) = (self.track(), self.disc());
        for y in (CY - R_OUT) as i32..=(CY + R_OUT) as i32 {
            for x in (cx - R_OUT) as i32..=(cx + R_OUT) as i32 {
                let dx = x as f32 - cx;
                let dy = y as f32 - CY;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= R_IN {
                    self.blend(x, y, disc, 235);
                } else if dist <= R_OUT {
                    let mut deg = dx.atan2(-dy).to_degrees();
                    if deg < 0.0 {
                        deg += 360.0;
                    }
                    let col = if deg <= fill_deg { fill } else { track };
                    self.blend(x, y, col, 255);
                }
            }
        }
    }

    fn number(&mut self, cx: i32, n: i32) {
        let label = n.to_string();
        let color = if self.frozen { (220, 238, 250) } else { (245, 245, 250) };
        let scale = if n >= 100 { 1 } else { 2 };
        let glyph_w = 5 * scale;
        let spacing = scale;
        let text_w = label.len() as i32 * glyph_w + (label.len() as i32 - 1) * spacing;
        let mut ox = cx - text_w / 2;
        let oy = CY as i32 - (7 * scale) / 2;
        for ch in label.chars() {
            if let Some(d) = ch.to_digit(10) {
                let pattern = &DIGITS[d as usize];
                for (row, bits) in pattern.iter().enumerate() {
                    for col in 0..5 {
                        if bits & (1 << (4 - col)) != 0 {
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    self.blend(ox + col * scale + sx, oy + row as i32 * scale + sy, color, 255);
                                }
                            }
                        }
                    }
                }
            }
            ox += glyph_w + spacing;
        }
    }

    /// Overlay animated flame tongues rising over the top of a ring.
    fn flames(&mut self, cx: f32, frame: u32) {
        let f = frame as f32;
        for fx in -15..=15 {
            let x = cx as i32 + fx;
            let edge = 1.0 - (fx.abs() as f32 / 16.0);
            let flick = 0.55 + 0.45 * ((fx as f32) * 1.1 + f * 0.8).sin();
            let wob = 0.85 + 0.15 * ((fx as f32) * 0.5 - f * 0.6).sin();
            let h = (R_OUT + 3.0) * edge * flick * wob;
            if h <= 0.0 {
                continue;
            }
            let base_y = (CY - 6.0) as i32;
            let steps = h as i32;
            for fy in 0..=steps {
                let y = base_y - fy;
                let t = fy as f32 / h.max(1.0);
                let col = flame_color(t);
                let a = (240.0 * (1.0 - t * 0.8)) as u8;
                self.blend(x, y, col, a);
            }
        }
    }

    /// Overlay frost: pale specks across the ring plus icicles hanging below it.
    fn frost(&mut self, cx: f32) {
        // sparse deterministic frost specks within the ring
        for y in (CY - R_OUT) as i32..=(CY + R_OUT) as i32 {
            for x in (cx - R_OUT) as i32..=(cx + R_OUT) as i32 {
                let dx = x as f32 - cx;
                let dy = y as f32 - CY;
                if (dx * dx + dy * dy).sqrt() > R_OUT {
                    continue;
                }
                if ((x * 7 + y * 13) % 17 == 0) || ((x * 5 - y * 3) % 23 == 0) {
                    self.blend(x, y, FROST, 150);
                }
            }
        }
        // icicles hanging from the bottom rim
        for (i, off) in [-9i32, -3, 4, 10].iter().enumerate() {
            let x = cx as i32 + off;
            let len = 3 + (i as i32 % 3);
            let base_y = (CY + R_OUT - 1.0) as i32;
            for d in 0..len {
                let a = (220 - d * 50).max(60) as u8;
                self.blend(x, base_y + d, FROST, a);
            }
        }
    }

    fn into_image(self) -> Image<'static> {
        Image::new_owned(self.buf, W as u32, H as u32)
    }
}

fn render(
    five: Option<f64>,
    seven: Option<f64>,
    warn: f64,
    crit: f64,
    flame_left: bool,
    flame_right: bool,
    frame: u32,
    frozen: bool,
) -> Image<'static> {
    let mut c = Canvas::new(frozen);

    let fv = five.unwrap_or(0.0);
    let left_fill = if frozen { ICE_FILL } else { escalate(fv, CORAL, warn, crit) };
    c.ring(LEFT_CX, fv, left_fill);
    // Frozen = data is stale; don't render the (stale) numbers.
    if five.is_some() && !frozen {
        c.number(LEFT_CX as i32, fv.round() as i32);
    }

    let sv = seven.unwrap_or(0.0);
    let right_fill = if frozen { ICE_FILL } else { escalate(sv, BLUE, warn, crit) };
    c.ring(RIGHT_CX, sv, right_fill);
    if seven.is_some() && !frozen {
        c.number(RIGHT_CX as i32, sv.round() as i32);
    }

    if frozen {
        c.frost(LEFT_CX);
        c.frost(RIGHT_CX);
    } else {
        if flame_left {
            c.flames(LEFT_CX, frame);
        }
        if flame_right {
            c.flames(RIGHT_CX, frame);
        }
    }

    c.into_image()
}

/// Static dual gauge (no flames, not frozen).
pub fn gauge_dual(five: Option<f64>, seven: Option<f64>, warn: f64, crit: f64) -> Image<'static> {
    render(five, seven, warn, crit, false, false, 0, false)
}

/// Dual gauge with flame overlay on the chosen ring(s) for the given frame.
pub fn gauge_dual_flame(
    five: Option<f64>,
    seven: Option<f64>,
    warn: f64,
    crit: f64,
    flame_left: bool,
    flame_right: bool,
    frame: u32,
) -> Image<'static> {
    render(five, seven, warn, crit, flame_left, flame_right, frame, false)
}

/// Frozen dual gauge — shown when the quota data is stale (token expired).
pub fn gauge_dual_frozen(five: Option<f64>, seven: Option<f64>) -> Image<'static> {
    render(five, seven, 0.0, 0.0, false, false, 0, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_have_expected_dimensions() {
        let img = gauge_dual(Some(50.0), Some(20.0), 75.0, 90.0);
        assert_eq!(img.width(), W as u32);
        assert_eq!(img.height(), H as u32);
        assert_eq!(img.rgba().len(), (W * H * 4) as usize);
    }

    #[test]
    fn frozen_and_flame_render_without_panic() {
        let _ = gauge_dual_frozen(Some(0.0), None);
        let _ = gauge_dual_flame(Some(100.0), Some(100.0), 75.0, 90.0, true, true, 5);
    }

    #[test]
    fn escalate_thresholds() {
        assert_eq!(escalate(50.0, CORAL, 75.0, 90.0), CORAL);
        assert_eq!(escalate(80.0, CORAL, 75.0, 90.0), (230, 177, 58));
        assert_eq!(escalate(95.0, CORAL, 75.0, 90.0), (230, 75, 58));
    }
}
