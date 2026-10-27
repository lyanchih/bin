use crate::config::ScaleSize;
use crate::font::{get_glyph, get_glyph_3x5};

#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_x11_pixel(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

pub struct Framebuffer {
    pub pixels: Vec<u32>,
    pub width: usize,
    pub height: usize,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            pixels: vec![0; width * height],
            width,
            height,
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.pixels.resize(width * height, 0);
    }

    pub fn clear(&mut self, color: Color) {
        let p = color.to_x11_pixel();
        self.pixels.fill(p);
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x >= 0 && (x as usize) < self.width && y >= 0 && (y as usize) < self.height {
            self.pixels[y as usize * self.width + x as usize] = color.to_x11_pixel();
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        for dy in 0..h {
            for dx in 0..w {
                self.set_pixel(x + dx, y + dy, color);
            }
        }
    }

    pub fn draw_rounded_capsule_outline(&mut self, color: Color) {
        let w = self.width as i32;
        let h = self.height as i32;
        let r = (w.min(h) / 2) as f64;

        for y in 0..h {
            for x in 0..w {
                let dist = if w >= h {
                    // Horizontal capsule
                    if (x as f64) < r {
                        let dx = x as f64 - r;
                        let dy = y as f64 - (h as f64 / 2.0);
                        (dx * dx + dy * dy).sqrt()
                    } else if (x as f64) >= (w as f64 - r) {
                        let dx = x as f64 - (w as f64 - r);
                        let dy = y as f64 - (h as f64 / 2.0);
                        (dx * dx + dy * dy).sqrt()
                    } else {
                        (y as f64 - (h as f64 / 2.0)).abs()
                    }
                } else {
                    // Vertical capsule
                    if (y as f64) < r {
                        let dx = x as f64 - (w as f64 / 2.0);
                        let dy = y as f64 - r;
                        (dx * dx + dy * dy).sqrt()
                    } else if (y as f64) >= (h as f64 - r) {
                        let dx = x as f64 - (w as f64 / 2.0);
                        let dy = y as f64 - (h as f64 - r);
                        (dx * dx + dy * dy).sqrt()
                    } else {
                        (x as f64 - (w as f64 / 2.0)).abs()
                    }
                };

                if dist >= (r - 1.2) && dist <= r {
                    self.set_pixel(x, y, color);
                }
            }
        }
    }

    /// Scaled 5x7 character rendering
    pub fn draw_char_scaled(&mut self, start_x: i32, start_y: i32, c: char, color: Color, scale: usize) {
        let glyph = get_glyph(c);
        let s = scale.max(1) as i32;
        for (row, &bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if (bits >> (4 - col)) & 1 == 1 {
                    for dy in 0..s {
                        for dx in 0..s {
                            self.set_pixel(start_x + col * s + dx, start_y + (row as i32) * s + dy, color);
                        }
                    }
                }
            }
        }
    }

    /// Scaled 5x7 text rendering
    pub fn draw_text_scaled(&mut self, start_x: i32, start_y: i32, text: &str, color: Color, scale: usize) {
        let s = scale.max(1) as i32;
        let char_spacing = 6 * s;
        let mut curr_x = start_x;
        for c in text.chars() {
            self.draw_char_scaled(curr_x, start_y, c, color, scale);
            curr_x += char_spacing;
        }
    }

    /// 3x5 text rendering for compact numbers and tooltips
    pub fn draw_text_3x5(&mut self, start_x: i32, start_y: i32, text: &str, color: Color) {
        let mut curr_x = start_x;
        for c in text.chars() {
            let glyph = get_glyph_3x5(c);
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..3 {
                    if (bits >> (2 - col)) & 1 == 1 {
                        self.set_pixel(curr_x + col, start_y + (row as i32), color);
                    }
                }
            }
            curr_x += 4; // 3px width + 1px spacing
        }
    }

    /// Draw circular ring progress gauge with centered number, properly scaled
    pub fn draw_ring_gauge(
        &mut self,
        cx: i32,
        cy: i32,
        pct: Option<f64>,
        agent_color: Color,
        scale: ScaleSize,
    ) {
        let (radius, thickness): (f64, f64) = match scale {
            ScaleSize::Small => (10.0, 2.0),
            ScaleSize::Normal => (13.5, 2.8),
            ScaleSize::Large => (21.0, 3.8),
        };

        let bg_ring_color = Color::rgb(35, 42, 54);
        let val = pct.unwrap_or(0.0).clamp(0.0, 100.0);

        let active_color = if val >= 90.0 {
            Color::rgb(244, 63, 94) // High alert pink-red
        } else if val >= 80.0 {
            Color::rgb(245, 158, 11) // Amber orange
        } else {
            agent_color
        };

        let r_in = radius - thickness;
        let r_out = radius;
        let box_r = (r_out + 1.0).ceil() as i32;
        let target_angle = (val / 100.0) * (std::f64::consts::PI * 2.0);

        for dy in -box_r..=box_r {
            for dx in -box_r..=box_r {
                let dist = ((dx * dx + dy * dy) as f64).sqrt();
                if dist >= r_in && dist <= r_out {
                    let mut angle = (-dx as f64).atan2(dy as f64) + std::f64::consts::PI;
                    if angle >= std::f64::consts::PI * 2.0 {
                        angle -= std::f64::consts::PI * 2.0;
                    }
                    if angle <= target_angle && val > 0.5 {
                        self.set_pixel(cx + dx, cy + dy, active_color);
                    } else {
                        self.set_pixel(cx + dx, cy + dy, bg_ring_color);
                    }
                }
            }
        }

        let text = match pct {
            Some(v) => format!("{:.0}%", v),
            None => "?%".to_string(),
        };

        match scale {
            ScaleSize::Small => {
                let text_width = (text.len() as i32) * 4 - 1;
                let text_height = 5;
                let tx = cx - text_width / 2;
                let ty = cy - text_height / 2;
                self.draw_text_3x5(tx, ty, &text, Color::rgb(255, 255, 255));
            }
            ScaleSize::Normal => {
                let text_width = (text.len() as i32) * 6 - 1;
                let text_height = 7;
                let tx = cx - text_width / 2;
                let ty = cy - text_height / 2;
                self.draw_text_scaled(tx, ty, &text, Color::rgb(255, 255, 255), 1);
            }
            ScaleSize::Large => {
                let text_width = (text.len() as i32) * 12 - 2;
                let text_height = 14;
                let tx = cx - text_width / 2;
                let ty = cy - text_height / 2;
                self.draw_text_scaled(tx, ty, &text, Color::rgb(255, 255, 255), 2);
            }
        }
    }

    /// Draw drag grip handle
    pub fn draw_drag_grip(&mut self, x: i32, y: i32, vertical: bool, scale: ScaleSize) {
        let grip_color = Color::rgb(70, 82, 105);
        let s = match scale {
            ScaleSize::Small => 1,
            ScaleSize::Normal => 1,
            ScaleSize::Large => 2,
        };
        if !vertical {
            for col in 0..2 {
                for row in 0..3 {
                    self.fill_rect(x + col * 3 * s, y + row * 4 * s, s, s, grip_color);
                }
            }
        } else {
            for col in 0..3 {
                for row in 0..2 {
                    self.fill_rect(x + col * 4 * s, y + row * 3 * s, s, s, grip_color);
                }
            }
        }
    }

    /// Claude Logo (scaled for Small: 9px, Normal: 13px, Large: 22px)
    pub fn draw_claude_logo(&mut self, x: i32, y: i32, scale: ScaleSize) {
        let c = Color::rgb(217, 119, 240);
        match scale {
            ScaleSize::Small => {
                self.fill_rect(x + 1, y + 6, 2, 3, c);
                self.fill_rect(x + 2, y + 4, 2, 2, c);
                self.fill_rect(x + 4, y + 1, 2, 3, c);
                self.fill_rect(x + 3, y + 5, 4, 2, c);
                self.fill_rect(x + 6, y + 4, 2, 2, c);
                self.fill_rect(x + 7, y + 6, 2, 3, c);
            }
            ScaleSize::Normal => {
                self.fill_rect(x + 1, y + 9, 2, 3, c);
                self.fill_rect(x + 3, y + 6, 2, 3, c);
                self.fill_rect(x + 5, y + 1, 3, 5, c);
                self.fill_rect(x + 4, y + 7, 5, 2, c);
                self.fill_rect(x + 8, y + 6, 2, 3, c);
                self.fill_rect(x + 10, y + 9, 2, 3, c);
            }
            ScaleSize::Large => {
                let s = 2;
                self.fill_rect(x + 1 * s, y + 8 * s, 2 * s, 3 * s, c);
                self.fill_rect(x + 3 * s, y + 5 * s, 2 * s, 3 * s, c);
                self.fill_rect(x + 5 * s, y + 1 * s, 2 * s, 4 * s, c);
                self.fill_rect(x + 4 * s, y + 7 * s, 5 * s, 2 * s, c);
                self.fill_rect(x + 7 * s, y + 5 * s, 2 * s, 3 * s, c);
                self.fill_rect(x + 9 * s, y + 8 * s, 2 * s, 3 * s, c);
            }
        }
    }

    /// Codex Logo (scaled for Small: 9px, Normal: 13px, Large: 22px)
    pub fn draw_codex_logo(&mut self, x: i32, y: i32, scale: ScaleSize) {
        let c = Color::rgb(16, 185, 129);
        match scale {
            ScaleSize::Small => {
                self.fill_rect(x + 3, y + 1, 3, 1, c);
                self.fill_rect(x + 1, y + 3, 1, 3, c);
                self.fill_rect(x + 7, y + 3, 1, 3, c);
                self.fill_rect(x + 3, y + 7, 3, 1, c);
                self.fill_rect(x + 4, y + 4, 1, 1, c);
                self.fill_rect(x + 2, y + 2, 1, 1, c);
                self.fill_rect(x + 6, y + 2, 1, 1, c);
                self.fill_rect(x + 2, y + 6, 1, 1, c);
                self.fill_rect(x + 6, y + 6, 1, 1, c);
            }
            ScaleSize::Normal => {
                self.fill_rect(x + 5, y + 1, 4, 2, c);
                self.fill_rect(x + 1, y + 5, 2, 4, c);
                self.fill_rect(x + 10, y + 5, 2, 4, c);
                self.fill_rect(x + 5, y + 10, 4, 2, c);
                self.fill_rect(x + 6, y + 6, 2, 2, c);
                self.fill_rect(x + 3, y + 3, 2, 2, c);
                self.fill_rect(x + 9, y + 3, 2, 2, c);
                self.fill_rect(x + 3, y + 9, 2, 2, c);
                self.fill_rect(x + 9, y + 9, 2, 2, c);
            }
            ScaleSize::Large => {
                let s = 2;
                self.fill_rect(x + 4 * s, y + 1 * s, 4 * s, 2 * s, c);
                self.fill_rect(x + 1 * s, y + 4 * s, 2 * s, 4 * s, c);
                self.fill_rect(x + 9 * s, y + 4 * s, 2 * s, 4 * s, c);
                self.fill_rect(x + 4 * s, y + 9 * s, 4 * s, 2 * s, c);
                self.fill_rect(x + 5 * s, y + 5 * s, 2 * s, 2 * s, c);
                self.fill_rect(x + 3 * s, y + 3 * s, s, s, c);
                self.fill_rect(x + 8 * s, y + 3 * s, s, s, c);
                self.fill_rect(x + 3 * s, y + 8 * s, s, s, c);
                self.fill_rect(x + 8 * s, y + 8 * s, s, s, c);
            }
        }
    }

    /// Gemini Logo (scaled for Small: 9px, Normal: 13px, Large: 22px)
    pub fn draw_gemini_logo(&mut self, x: i32, y: i32, scale: ScaleSize) {
        let c = Color::rgb(59, 130, 246);
        match scale {
            ScaleSize::Small => {
                let cx = x + 4;
                let cy = y + 4;
                for i in -4..=4 {
                    self.set_pixel(cx + i, cy, c);
                    self.set_pixel(cx, cy + i, c);
                }
                self.set_pixel(cx - 1, cy - 1, c);
                self.set_pixel(cx + 1, cy - 1, c);
                self.set_pixel(cx - 1, cy + 1, c);
                self.set_pixel(cx + 1, cy + 1, c);
            }
            ScaleSize::Normal => {
                let cx = x + 6;
                let cy = y + 6;
                for i in -6..=6 {
                    self.set_pixel(cx + i, cy, c);
                    self.set_pixel(cx, cy + i, c);
                }
                self.fill_rect(cx - 1, cy - 1, 3, 3, c);
                self.set_pixel(cx - 2, cy - 1, c);
                self.set_pixel(cx + 2, cy - 1, c);
                self.set_pixel(cx - 2, cy + 1, c);
                self.set_pixel(cx + 2, cy + 1, c);
            }
            ScaleSize::Large => {
                let s = 2;
                let cx = x + 5 * s;
                let cy = y + 5 * s;
                for i in -5..=5 {
                    self.fill_rect(cx + i * s, cy, s, s, c);
                    self.fill_rect(cx, cy + i * s, s, s, c);
                }
                self.fill_rect(cx - 1 * s, cy - 1 * s, s, s, c);
                self.fill_rect(cx + 1 * s, cy - 1 * s, s, s, c);
                self.fill_rect(cx - 1 * s, cy + 1 * s, s, s, c);
                self.fill_rect(cx + 1 * s, cy + 1 * s, s, s, c);
            }
        }
    }

    /// Cursor Logo (mouse pointer arrow in cyan, scaled for Small: 9px, Normal: 13px, Large: 22px)
    pub fn draw_cursor_logo(&mut self, x: i32, y: i32, scale: ScaleSize) {
        let c = Color::rgb(0, 212, 255);
        match scale {
            ScaleSize::Small => {
                self.fill_rect(x + 1, y + 1, 1, 1, c);
                self.fill_rect(x + 1, y + 2, 2, 1, c);
                self.fill_rect(x + 1, y + 3, 3, 1, c);
                self.fill_rect(x + 1, y + 4, 4, 1, c);
                self.fill_rect(x + 1, y + 5, 5, 1, c);
                self.fill_rect(x + 1, y + 6, 3, 1, c);
                self.fill_rect(x + 1, y + 7, 2, 1, c);
                self.fill_rect(x + 4, y + 6, 2, 1, c);
                self.fill_rect(x + 5, y + 7, 2, 1, c);
            }
            ScaleSize::Normal => {
                self.fill_rect(x + 2, y + 1, 2, 1, c);
                self.fill_rect(x + 2, y + 2, 3, 1, c);
                self.fill_rect(x + 2, y + 3, 4, 1, c);
                self.fill_rect(x + 2, y + 4, 5, 1, c);
                self.fill_rect(x + 2, y + 5, 6, 1, c);
                self.fill_rect(x + 2, y + 6, 7, 1, c);
                self.fill_rect(x + 2, y + 7, 8, 1, c);
                self.fill_rect(x + 2, y + 8, 4, 1, c);
                self.fill_rect(x + 2, y + 9, 3, 1, c);
                self.fill_rect(x + 6, y + 8, 2, 2, c);
                self.fill_rect(x + 7, y + 10, 2, 2, c);
            }
            ScaleSize::Large => {
                let s = 2;
                self.fill_rect(x + 1 * s, y + 1 * s, 1 * s, 1 * s, c);
                self.fill_rect(x + 1 * s, y + 2 * s, 2 * s, 1 * s, c);
                self.fill_rect(x + 1 * s, y + 3 * s, 3 * s, 1 * s, c);
                self.fill_rect(x + 1 * s, y + 4 * s, 4 * s, 1 * s, c);
                self.fill_rect(x + 1 * s, y + 5 * s, 5 * s, 1 * s, c);
                self.fill_rect(x + 1 * s, y + 6 * s, 3 * s, 1 * s, c);
                self.fill_rect(x + 1 * s, y + 7 * s, 2 * s, 1 * s, c);
                self.fill_rect(x + 4 * s, y + 6 * s, 2 * s, 1 * s, c);
                self.fill_rect(x + 5 * s, y + 7 * s, 2 * s, 1 * s, c);
            }
        }
    }

    pub fn draw_divider_v(&mut self, x: i32) {
        let c = Color::rgb(28, 36, 48);
        let pad = (self.height as i32 / 5).max(4);
        for y in pad..(self.height as i32 - pad) {
            self.set_pixel(x, y, c);
        }
    }

    pub fn draw_divider_h(&mut self, y: i32) {
        let c = Color::rgb(28, 36, 48);
        let pad = (self.width as i32 / 5).max(4);
        for x in pad..(self.width as i32 - pad) {
            self.set_pixel(x, y, c);
        }
    }

    // --- Hover Action Buttons ---

    pub fn draw_icon_orientation(&mut self, x: i32, y: i32, is_hover: bool, scale: ScaleSize) {
        let c = if is_hover {
            Color::rgb(96, 165, 250)
        } else {
            Color::rgb(148, 163, 184)
        };
        let (dot, gap) = match scale {
            ScaleSize::Small => (2, 2),
            ScaleSize::Normal => (3, 2),
            ScaleSize::Large => (4, 3),
        };
        self.fill_rect(x, y, dot, dot, c);
        self.fill_rect(x + dot + gap, y, dot, dot, c);
        self.fill_rect(x, y + dot + gap, dot, dot, c);
        self.fill_rect(x + dot + gap, y + dot + gap, dot, dot, c);
    }

    pub fn draw_icon_resize(&mut self, x: i32, y: i32, is_hover: bool, scale: ScaleSize) {
        let c = if is_hover {
            Color::rgb(96, 165, 250)
        } else {
            Color::rgb(148, 163, 184)
        };
        match scale {
            ScaleSize::Small => {
                self.fill_rect(x, y, 3, 1, c);
                self.fill_rect(x, y, 1, 3, c);
                self.fill_rect(x + 3, y + 5, 3, 1, c);
                self.fill_rect(x + 5, y + 3, 1, 3, c);
                self.set_pixel(x + 2, y + 2, c);
                self.set_pixel(x + 3, y + 3, c);
            }
            ScaleSize::Normal => {
                self.fill_rect(x, y, 4, 1, c);
                self.fill_rect(x, y, 1, 4, c);
                self.fill_rect(x + 4, y + 7, 4, 1, c);
                self.fill_rect(x + 7, y + 4, 1, 4, c);
                self.set_pixel(x + 3, y + 3, c);
                self.set_pixel(x + 4, y + 4, c);
            }
            ScaleSize::Large => {
                let s = 2;
                self.fill_rect(x, y, 3 * s, 1 * s, c);
                self.fill_rect(x, y, 1 * s, 3 * s, c);
                self.fill_rect(x + 3 * s, y + 5 * s, 3 * s, 1 * s, c);
                self.fill_rect(x + 5 * s, y + 3 * s, 1 * s, 3 * s, c);
                self.fill_rect(x + 2 * s, y + 2 * s, s, s, c);
                self.fill_rect(x + 3 * s, y + 3 * s, s, s, c);
            }
        }
    }

    pub fn draw_icon_settings(&mut self, x: i32, y: i32, is_hover: bool, scale: ScaleSize) {
        let c = if is_hover {
            Color::rgb(251, 191, 36) // Warm amber highlight
        } else {
            Color::rgb(148, 163, 184)
        };
        match scale {
            ScaleSize::Small => {
                self.fill_rect(x + 2, y, 3, 1, c);
                self.fill_rect(x + 2, y + 6, 3, 1, c);
                self.fill_rect(x, y + 2, 1, 3, c);
                self.fill_rect(x + 6, y + 2, 1, 3, c);
                self.fill_rect(x + 1, y + 1, 5, 5, c);
                self.set_pixel(x + 3, y + 3, Color::rgb(12, 16, 23));
            }
            ScaleSize::Normal => {
                self.fill_rect(x + 3, y, 4, 1, c);
                self.fill_rect(x + 3, y + 9, 4, 1, c);
                self.fill_rect(x, y + 3, 1, 4, c);
                self.fill_rect(x + 9, y + 3, 1, 4, c);
                self.fill_rect(x + 2, y + 2, 6, 6, c);
                self.fill_rect(x + 4, y + 4, 2, 2, Color::rgb(12, 16, 23));
                self.set_pixel(x + 1, y + 1, c);
                self.set_pixel(x + 8, y + 1, c);
                self.set_pixel(x + 1, y + 8, c);
                self.set_pixel(x + 8, y + 8, c);
            }
            ScaleSize::Large => {
                let s = 2;
                self.fill_rect(x + 2 * s, y, 3 * s, 1 * s, c);
                self.fill_rect(x + 2 * s, y + 6 * s, 3 * s, 1 * s, c);
                self.fill_rect(x, y + 2 * s, 1 * s, 3 * s, c);
                self.fill_rect(x + 6 * s, y + 2 * s, 1 * s, 3 * s, c);
                self.fill_rect(x + 1 * s, y + 1 * s, 5 * s, 5 * s, c);
                self.fill_rect(x + 3 * s - 1, y + 3 * s - 1, 2 * s, 2 * s, Color::rgb(12, 16, 23));
            }
        }
    }

    pub fn draw_icon_refresh(&mut self, x: i32, y: i32, is_hover: bool, scale: ScaleSize) {
        let c = if is_hover {
            Color::rgb(52, 211, 153)
        } else {
            Color::rgb(148, 163, 184)
        };
        match scale {
            ScaleSize::Small => {
                self.fill_rect(x + 2, y, 3, 1, c);
                self.fill_rect(x + 5, y + 1, 1, 3, c);
                self.fill_rect(x + 2, y + 5, 3, 1, c);
                self.fill_rect(x, y + 2, 1, 3, c);
                self.fill_rect(x, y, 2, 1, c);
            }
            ScaleSize::Normal => {
                self.fill_rect(x + 2, y, 4, 1, c);
                self.fill_rect(x + 6, y + 1, 1, 4, c);
                self.fill_rect(x + 2, y + 6, 4, 1, c);
                self.fill_rect(x, y + 3, 1, 3, c);
                self.fill_rect(x, y, 3, 1, c);
                self.fill_rect(x + 1, y + 1, 1, 2, c);
            }
            ScaleSize::Large => {
                let s = 2;
                self.fill_rect(x + 2 * s, y, 3 * s, 1 * s, c);
                self.fill_rect(x + 5 * s, y + 1 * s, 1 * s, 3 * s, c);
                self.fill_rect(x + 2 * s, y + 5 * s, 3 * s, 1 * s, c);
                self.fill_rect(x, y + 2 * s, 1 * s, 3 * s, c);
                self.fill_rect(x, y, 2 * s, 1 * s, c);
            }
        }
    }

    pub fn draw_icon_close(&mut self, x: i32, y: i32, is_hover: bool, scale: ScaleSize) {
        let c = if is_hover {
            Color::rgb(244, 63, 94)
        } else {
            Color::rgb(148, 163, 184)
        };
        let len = match scale {
            ScaleSize::Small => 6,
            ScaleSize::Normal => 7,
            ScaleSize::Large => 11,
        };
        for i in 0..len {
            self.set_pixel(x + i, y + i, c);
            self.set_pixel(x + len - 1 - i, y + i, c);
        }
    }

    /// Draw a floating tooltip showing the agent name and reset countdown
    pub fn draw_tooltip(&mut self, x: i32, y: i32, text: &str, scale: ScaleSize) {
        let (glyph_w, glyph_h, pixel_scale, pad) = match scale {
            ScaleSize::Small => (3, 5, 1, 3),
            ScaleSize::Normal => (5, 7, 1, 4),
            ScaleSize::Large => (3, 5, 2, 5),
        };
        let advance = (glyph_w + 1) * pixel_scale;
        let max_chars = ((self.width as i32 - 4 - pad * 2 + pixel_scale) / advance).max(1) as usize;
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max_chars {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() { line.push(' '); }
            for c in word.chars() {
                if line.chars().count() == max_chars {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        if !line.is_empty() { lines.push(line); }
        let text_w = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0) as i32
            * advance - pixel_scale;
        let line_height = (glyph_h + 1) * pixel_scale;
        let box_w = text_w.max(0) + pad * 2;
        let box_h = lines.len() as i32 * line_height - pixel_scale + 4;
        let bx = (x - box_w / 2).clamp(2, (self.width as i32 - box_w - 2).max(2));
        let by = (y - box_h).clamp(2, (self.height as i32 - box_h - 2).max(2));
        self.fill_rect(bx, by, box_w, box_h, Color::rgb(71, 85, 105));
        self.fill_rect(bx + 1, by + 1, box_w - 2, box_h - 2, Color::rgb(24, 32, 47));
        for (i, line) in lines.iter().enumerate() {
            for (j, c) in line.chars().enumerate() {
                let glyph: &[u8] = if glyph_w == 3 { get_glyph_3x5(c) } else { get_glyph(c) };
                for (row, &bits) in glyph.iter().enumerate() {
                    for col in 0..glyph_w {
                        if (bits >> (glyph_w - col - 1)) & 1 == 1 {
                            self.fill_rect(
                                bx + pad + j as i32 * advance + col * pixel_scale,
                                by + 2 + i as i32 * line_height + row as i32 * pixel_scale,
                                pixel_scale, pixel_scale, Color::rgb(241, 245, 249),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Checkbox item for Settings window
    pub fn draw_checkbox_row(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        checked: bool,
        is_hover: bool,
        name: &str,
        agent_color: Color,
    ) {
        if is_hover {
            self.fill_rect(x, y, w, h, Color::rgb(30, 41, 59));
        }

        // Checkbox square (12x12)
        let cb_size = 12;
        let cb_x = x + 10;
        let cb_y = y + (h - cb_size) / 2;

        if checked {
            self.fill_rect(cb_x, cb_y, cb_size, cb_size, agent_color);
            // White checkmark
            let tick_color = Color::rgb(255, 255, 255);
            self.set_pixel(cb_x + 3, cb_y + 6, tick_color);
            self.set_pixel(cb_x + 4, cb_y + 7, tick_color);
            self.set_pixel(cb_x + 5, cb_y + 8, tick_color);
            self.set_pixel(cb_x + 6, cb_y + 7, tick_color);
            self.set_pixel(cb_x + 7, cb_y + 5, tick_color);
            self.set_pixel(cb_x + 8, cb_y + 4, tick_color);
            self.set_pixel(cb_x + 9, cb_y + 3, tick_color);
        } else {
            for dx in 0..cb_size {
                self.set_pixel(cb_x + dx, cb_y, Color::rgb(71, 85, 105));
                self.set_pixel(cb_x + dx, cb_y + cb_size - 1, Color::rgb(71, 85, 105));
            }
            for dy in 0..cb_size {
                self.set_pixel(cb_x, cb_y + dy, Color::rgb(71, 85, 105));
                self.set_pixel(cb_x + cb_size - 1, cb_y + dy, Color::rgb(71, 85, 105));
            }
        }

        // Agent color dot indicator
        self.fill_rect(cb_x + 20, y + (h - 6) / 2, 6, 6, agent_color);

        // Name
        let text_color = if checked {
            Color::rgb(241, 245, 249)
        } else {
            Color::rgb(148, 163, 184)
        };
        self.draw_text_scaled(cb_x + 32, y + (h - 7) / 2, name, text_color, 1);
    }
}
