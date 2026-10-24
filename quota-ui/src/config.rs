use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleSize {
    Small,  // 0.85x
    Normal, // 1.0x
    Large,  // 1.25x
}

impl ScaleSize {
    pub fn next(self) -> Self {
        match self {
            ScaleSize::Small => ScaleSize::Normal,
            ScaleSize::Normal => ScaleSize::Large,
            ScaleSize::Large => ScaleSize::Small,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub x: i32,
    pub y: i32,
    pub orientation: Orientation,
    pub scale: ScaleSize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            x: 100,
            y: 32,
            orientation: Orientation::Horizontal,
            scale: ScaleSize::Normal,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("quota-ui")
            .join("config.json"),
    )
}

pub fn load_config() -> AppConfig {
    let mut cfg = AppConfig::default();
    let Some(path) = config_path() else {
        return cfg;
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return cfg;
    };
    let Ok(doc) = crate::json::parse(&raw) else {
        return cfg;
    };

    if let Some(x) = doc.get("x").and_then(|v| v.as_i64()) {
        cfg.x = x as i32;
    }
    if let Some(y) = doc.get("y").and_then(|v| v.as_i64()) {
        cfg.y = y as i32;
    }
    if let Some(orient) = doc.get("orientation").and_then(|v| v.as_str()) {
        if orient == "vertical" {
            cfg.orientation = Orientation::Vertical;
        }
    }
    if let Some(sc) = doc.get("scale").and_then(|v| v.as_str()) {
        cfg.scale = match sc {
            "small" => ScaleSize::Small,
            "large" => ScaleSize::Large,
            _ => ScaleSize::Normal,
        };
    }

    cfg
}

pub fn save_config(cfg: &AppConfig) {
    if let Some(path) = config_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let orient_str = match cfg.orientation {
            Orientation::Horizontal => "horizontal",
            Orientation::Vertical => "vertical",
        };
        let scale_str = match cfg.scale {
            ScaleSize::Small => "small",
            ScaleSize::Normal => "normal",
            ScaleSize::Large => "large",
        };
        let json_str = format!(
            "{{\n  \"x\": {},\n  \"y\": {},\n  \"orientation\": \"{}\",\n  \"scale\": \"{}\"\n}}\n",
            cfg.x, cfg.y, orient_str, scale_str
        );
        let _ = fs::write(path, json_str);
    }
}
