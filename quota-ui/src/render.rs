use crate::font::get_glyph;

pub const WIDTH: usize = 236;
pub const HEIGHT: usize = 38;

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

    pub fn lerp(self, other: Color, t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        Color {
            r: (self.r as f64 + (other.r as f64 - self.r as f64) * t).round() as u8,
            g: (self.g as f64 + (other.g as f64 - self.g as f64) * t).round() as u8,
            b: (self.b as f64 + (other.b as f64 - self.b as f64) * t).round() as u8,
        }
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
        let r = h / 2;

        for y in 0..h {
            for x in 0..w {
                let dist_from_center = if x < r {
                    let dx = x - r;
                    let dy = y - r;
                    ((dx * dx + dy * dy) as f64).sqrt()
                } else if x >= w - r {
                    let dx = x - (w - r);
                    let dy = y - r;
                    ((dx * dx + dy * dy) as f64).sqrt()
                } else {
                    (y - r).abs() as f64
                };

                if dist_from_center >= (r as f64 - 1.2) && dist_from_center <= (r as f64) {
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

        // Ring active color changes by usage:
        // < 80%: agent_color or green
        // 80..90%: amber orange
        // >= 90%: high-alert pink-red (#f43f5e)
        let active_color = if val >= 90.0 {
            Color::rgb(244, 63, 94) // Pinkish Red (like user's screenshot)
        } else if val >= 80.0 {
            Color::rgb(245, 158, 11) // Amber
        } else {
            agent_color
        };

        let r_in = radius - thickness;
        let r_out = radius;
        let box_r = (r_out + 1.0).ceil() as i32;

        // Angle 0 is at 12 o'clock (top), rotating clockwise
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

        // Draw percentage text inside the ring
        let text = match pct {
            Some(v) => format!("{:.0}%", v),
            None => "?%".to_string(),
        };

        let text_width = (text.len() * 6 - 1) as i32;
        let tx = cx - text_width / 2;
        let ty = cy - 3;
        self.draw_text(tx, ty, &text, Color::rgb(255, 255, 255));
    }

    /// Draw 6-dot drag grip handle on the far left
    pub fn draw_drag_grip(&mut self, x: i32, y: i32) {
        let grip_color = Color::rgb(70, 82, 105);
        for col in 0..2 {
            for row in 0..3 {
                self.set_pixel(x + col * 3, y + row * 4, grip_color);
            }
        }
    }

    /// Draw Claude minimalist Logo (stylized geometric 'A\')
    pub fn draw_claude_logo(&mut self, x: i32, y: i32) {
        let c = Color::rgb(217, 119, 240); // Claude Coral Purple
        // Left slant '/'
        self.fill_rect(x + 1, y + 8, 2, 3, c);
        self.fill_rect(x + 3, y + 5, 2, 3, c);
        self.fill_rect(x + 5, y + 1, 2, 4, c);
        // Middle crossbar
        self.fill_rect(x + 4, y + 7, 5, 2, c);
        // Right slant '\'
        self.fill_rect(x + 7, y + 5, 2, 3, c);
        self.fill_rect(x + 9, y + 8, 2, 3, c);
    }

    /// Draw OpenAI Codex minimalist Logo (swirling geometric rosette)
    pub fn draw_codex_logo(&mut self, x: i32, y: i32) {
        let c = Color::rgb(16, 185, 129); // Codex Green
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

    /// Draw Google Gemini minimalist Logo (4-pointed star sparkle)
    pub fn draw_gemini_logo(&mut self, x: i32, y: i32) {
        let c = Color::rgb(59, 130, 246); // Gemini Blue
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

    /// Draw subtle vertical divider
    pub fn draw_divider(&mut self, x: i32) {
        let c = Color::rgb(28, 36, 48);
        for y in 8..(self.height as i32 - 8) {
            self.set_pixel(x, y, c);
        }
    }
}
