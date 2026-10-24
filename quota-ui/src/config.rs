use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

fn config_path() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("quota-ui")
            .join("position.json"),
    )
}

pub fn load_position() -> Option<Position> {
    let path = config_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let doc = crate::json::parse(&raw).ok()?;
    let x = doc.get("x")?.as_i64()? as i32;
    let y = doc.get("y")?.as_i64()? as i32;
    Some(Position { x, y })
}

pub fn save_position(x: i32, y: i32) {
    if let Some(path) = config_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let json_str = format!("{{\"x\": {x}, \"y\": {y}}}\n");
        let _ = fs::write(path, json_str);
    }
}
