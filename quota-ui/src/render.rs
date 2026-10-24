use crate::font::get_glyph;

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

    pub fn fill_rect(&mut self, x: i32, y: i32, w: usize, h: usize, color: Color) {
        for dy in 0..(h as i32) {
            for dx in 0..(w as i32) {
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

    pub fn draw_char(&mut self, start_x: i32, start_y: i32, c: char, color: Color) {
        let glyph = get_glyph(c);
        for (row, &bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if (bits >> (4 - col)) & 1 == 1 {
                    self.set_pixel(start_x + col, start_y + row as i32, color);
                }
            }
        }
    }

    pub fn draw_text(&mut self, start_x: i32, start_y: i32, text: &str, color: Color) {
        let mut curr_x = start_x;
        for c in text.chars() {
            self.draw_char(curr_x, start_y, c, color);
            curr_x += 6;
        }
    }

    /// Draw circular ring progress gauge with centered number
    pub fn draw_ring_gauge(
        &mut self,
        cx: i32,
        cy: i32,
        radius: f64,
        thickness: f64,
        pct: Option<f64>,
        agent_color: Color,
    ) {
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

        let text_width = (text.len() * 6 - 1) as i32;
        let tx = cx - text_width / 2;
        let ty = cy - 3;
        self.draw_text(tx, ty, &text, Color::rgb(255, 255, 255));
    }

    /// Draw drag grip handle
    pub fn draw_drag_grip(&mut self, x: i32, y: i32, vertical: bool) {
        let grip_color = Color::rgb(70, 82, 105);
        if !vertical {
            for col in 0..2 {
                for row in 0..3 {
                    self.set_pixel(x + col * 3, y + row * 4, grip_color);
                }
            }
        } else {
            for col in 0..3 {
                for row in 0..2 {
                    self.set_pixel(x + col * 4, y + row * 3, grip_color);
                }
            }
        }
    }

    /// Claude Logo
    pub fn draw_claude_logo(&mut self, x: i32, y: i32) {
        let c = Color::rgb(217, 119, 240);
        self.fill_rect(x + 1, y + 8, 2, 3, c);
        self.fill_rect(x + 3, y + 5, 2, 3, c);
        self.fill_rect(x + 5, y + 1, 2, 4, c);
        self.fill_rect(x + 4, y + 7, 5, 2, c);
        self.fill_rect(x + 7, y + 5, 2, 3, c);
        self.fill_rect(x + 9, y + 8, 2, 3, c);
    }

    /// Codex Logo
    pub fn draw_codex_logo(&mut self, x: i32, y: i32) {
        let c = Color::rgb(16, 185, 129);
        self.fill_rect(x + 4, y + 1, 4, 2, c);
        self.fill_rect(x + 1, y + 4, 2, 4, c);
        self.fill_rect(x + 9, y + 4, 2, 4, c);
        self.fill_rect(x + 4, y + 9, 4, 2, c);
        self.fill_rect(x + 5, y + 5, 2, 2, c);
        self.set_pixel(x + 3, y + 3, c);
        self.set_pixel(x + 8, y + 3, c);
        self.set_pixel(x + 3, y + 8, c);
        self.set_pixel(x + 8, y + 8, c);
    }

    /// Gemini Logo
    pub fn draw_gemini_logo(&mut self, x: i32, y: i32) {
        let c = Color::rgb(59, 130, 246);
        let cx = x + 5;
        let cy = y + 5;
        for i in -5..=5 {
            self.set_pixel(cx + i, cy, c);
            self.set_pixel(cx, cy + i, c);
        }
        self.set_pixel(cx - 1, cy - 1, c);
        self.set_pixel(cx + 1, cy - 1, c);
        self.set_pixel(cx - 1, cy + 1, c);
        self.set_pixel(cx + 1, cy + 1, c);
    }

    pub fn draw_divider_v(&mut self, x: i32) {
        let c = Color::rgb(28, 36, 48);
        for y in 8..(self.height as i32 - 8) {
            self.set_pixel(x, y, c);
        }
    }

    pub fn draw_divider_h(&mut self, y: i32) {
        let c = Color::rgb(28, 36, 48);
        for x in 8..(self.width as i32 - 8) {
            self.set_pixel(x, y, c);
        }
    }

    // --- Hover Action Buttons ---

    /// Icon 1: 排列 (Orientation toggle) - 2x2 grid icon
    pub fn draw_icon_orientation(&mut self, x: i32, y: i32, is_hover: bool) {
        let c = if is_hover {
            Color::rgb(96, 165, 250)
        } else {
            Color::rgb(148, 163, 184)
        };
        self.fill_rect(x, y, 3, 3, c);
        self.fill_rect(x + 5, y, 3, 3, c);
        self.fill_rect(x, y + 5, 3, 3, c);
        self.fill_rect(x + 5, y + 5, 3, 3, c);
    }

    /// Icon 2: 縮放 (Resize/Scale toggle) - Diagonal arrows / expanding square
    pub fn draw_icon_resize(&mut self, x: i32, y: i32, is_hover: bool) {
        let c = if is_hover {
            Color::rgb(96, 165, 250)
        } else {
            Color::rgb(148, 163, 184)
        };
        // Top-left corner
        self.fill_rect(x, y, 4, 1, c);
        self.fill_rect(x, y, 1, 4, c);
        // Bottom-right corner
        self.fill_rect(x + 4, y + 7, 4, 1, c);
        self.fill_rect(x + 7, y + 4, 1, 4, c);
        // Center diagonal dot
        self.set_pixel(x + 3, y + 3, c);
        self.set_pixel(x + 4, y + 4, c);
    }

    /// Icon 3: 重整 (Refresh) - Circular arrow
    pub fn draw_icon_refresh(&mut self, x: i32, y: i32, is_hover: bool) {
        let c = if is_hover {
            Color::rgb(52, 211, 153)
        } else {
            Color::rgb(148, 163, 184)
        };
        // Arc
        self.fill_rect(x + 2, y, 4, 1, c);
        self.fill_rect(x + 6, y + 1, 1, 4, c);
        self.fill_rect(x + 2, y + 6, 4, 1, c);
        self.fill_rect(x, y + 3, 1, 3, c);
        // Arrow head
        self.fill_rect(x, y, 3, 1, c);
        self.fill_rect(x + 1, y + 1, 1, 2, c);
    }

    /// Icon 4: 關閉 (Close) - '✕' cross
    pub fn draw_icon_close(&mut self, x: i32, y: i32, is_hover: bool) {
        let c = if is_hover {
            Color::rgb(244, 63, 94)
        } else {
            Color::rgb(148, 163, 184)
        };
        for i in 0..7 {
            self.set_pixel(x + i, y + i, c);
            self.set_pixel(x + 6 - i, y + i, c);
        }
    }
}
