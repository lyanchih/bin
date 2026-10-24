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
    Small,  // Compact
    Normal, // Standard
    Large,  // Large
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

pub const ALL_AGENTS: [&str; 4] = ["claude", "codex", "gemini", "cursor"];

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub x: i32,
    pub y: i32,
    pub orientation: Orientation,
    pub scale: ScaleSize,
    pub enabled_agents: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            x: 100,
            y: 32,
            orientation: Orientation::Horizontal,
            scale: ScaleSize::Normal,
            enabled_agents: ALL_AGENTS.iter().map(|&s| s.to_string()).collect(),
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
    if let Some(agents_arr) = doc.get("enabled_agents").and_then(|v| v.as_array()) {
        let mut loaded = Vec::new();
        for item in agents_arr {
            if let Some(name) = item.as_str() {
                if ALL_AGENTS.contains(&name) && !loaded.contains(&name.to_string()) {
                    loaded.push(name.to_string());
                }
            }
        }
        if !loaded.is_empty() {
            cfg.enabled_agents = loaded;
        }
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
        let agents_json: Vec<String> = cfg
            .enabled_agents
            .iter()
            .map(|s| format!("    \"{}\"", s))
            .collect();
        let json_str = format!(
            "{{\n  \"x\": {},\n  \"y\": {},\n  \"orientation\": \"{}\",\n  \"scale\": \"{}\",\n  \"enabled_agents\": [\n{}\n  ]\n}}\n",
            cfg.x,
            cfg.y,
            orient_str,
            scale_str,
            agents_json.join(",\n")
        );
        let _ = fs::write(path, json_str);
    }
}
