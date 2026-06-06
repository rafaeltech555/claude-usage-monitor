//! Dynamically rendered tray icon: two ring gauges side by side —
//! left = 5-hour window (current, coral), right = weekly (blue) — each filled
//! by its utilization with the number in the center. When usage rises, an
//! optional flame effect is overlaid on the affected ring (animated by frame).

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
}

impl Canvas {
    fn new() -> Self {
        Canvas {
            buf: vec![0u8; (W * H * 4) as usize],
        }
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
        for y in (CY - R_OUT) as i32..=(CY + R_OUT) as i32 {
            for x in (cx - R_OUT) as i32..=(cx + R_OUT) as i32 {
                let dx = x as f32 - cx;
                let dy = y as f32 - CY;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= R_IN {
                    self.blend(x, y, DISC, 235);
                } else if dist <= R_OUT {
                    let mut deg = dx.atan2(-dy).to_degrees();
                    if deg < 0.0 {
                        deg += 360.0;
                    }
                    let col = if deg <= fill_deg { fill } else { TRACK };
                    self.blend(x, y, col, 255);
                }
            }
        }
    }

    fn number(&mut self, cx: i32, n: i32) {
        let label = n.to_string();
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
                                    self.blend(ox + col * scale + sx, oy + row as i32 * scale + sy, (245, 245, 250), 255);
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
            let base_y = (CY - 6.0) as i32; // start above the number, over the top arc
            let steps = h as i32;
            for fy in 0..=steps {
                let y = base_y - fy; // rise upward
                let t = fy as f32 / h.max(1.0); // 0 at base, 1 at tip
                let col = flame_color(t);
                let a = (240.0 * (1.0 - t * 0.8)) as u8;
                self.blend(x, y, col, a);
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
) -> Image<'static> {
    let mut c = Canvas::new();

    let fv = five.unwrap_or(0.0);
    c.ring(LEFT_CX, fv, escalate(fv, CORAL, warn, crit));
    if five.is_some() {
        c.number(LEFT_CX as i32, fv.round() as i32);
    }
    if flame_left {
        c.flames(LEFT_CX, frame);
    }

    let sv = seven.unwrap_or(0.0);
    c.ring(RIGHT_CX, sv, escalate(sv, BLUE, warn, crit));
    if seven.is_some() {
        c.number(RIGHT_CX as i32, sv.round() as i32);
    }
    if flame_right {
        c.flames(RIGHT_CX, frame);
    }

    c.into_image()
}

/// Static dual gauge (no flames).
pub fn gauge_dual(five: Option<f64>, seven: Option<f64>, warn: f64, crit: f64) -> Image<'static> {
    render(five, seven, warn, crit, false, false, 0)
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
    render(five, seven, warn, crit, flame_left, flame_right, frame)
}
