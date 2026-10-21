//! The theme, saved in `config_dir()/sc/config` as a `key=value` line.
use crate::reader::Theme;
use std::path::PathBuf;

pub struct Config {
    pub theme: Theme,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join(crate::NAME).join("config"))
}

pub fn load() -> Config {
    let mut config = Config { theme: Theme::Dark };
    let Some(content) = config_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return config;
    };
    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "theme" {
                if let Some(theme) = Theme::parse(value.trim()) {
                    config.theme = theme;
                }
            }
        }
    }
    config
}

pub fn save(theme: Theme) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("theme={}\n", theme.name()));
}
