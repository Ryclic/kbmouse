use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub leader: String,
    pub hold_leader_for_normal: bool,
    pub leader_tap_ms: u64,
    pub label_style: LabelStyle,
    pub alphabet: String,
    pub grid_rows: Option<usize>,
    pub grid_cols: Option<usize>,
    pub target_cell_px: u32,
    pub backdrop_opacity: u8,
    pub background_color: String,
    pub grid_color: String,
    pub text_color: String,
    pub accent_color: String,
    pub high_contrast_labels: bool,
    pub crisp_labels: bool,
    pub label_glow: bool,
    pub font_size: u32,
    pub post_hint: PostHint,
    pub exit_on_click: bool,
    pub move_step: i32,
    pub hold_move_step: i32,
    pub smooth_movement: bool,
    pub scroll_step: i32,
    pub span_all_monitors: bool,
    pub keys: Keys,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PostHint {
    Normal,
    Click,
    Exit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LabelStyle {
    Sequences,
    Words,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub left: String,
    pub down: String,
    pub up: String,
    pub right: String,
    pub left_click: String,
    pub middle_click: String,
    pub right_click: String,
    pub drag: String,
    pub scroll_up: String,
    pub scroll_down: String,
    pub subdivide: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            leader: "capslock".into(),
            hold_leader_for_normal: true,
            leader_tap_ms: 200,
            label_style: LabelStyle::Sequences,
            alphabet: "asdfghjkl;qwertyuiop".into(),
            grid_rows: None,
            grid_cols: None,
            target_cell_px: 100,
            backdrop_opacity: 90,
            background_color: "#111827".into(),
            grid_color: "#64748b".into(),
            text_color: "#ffffff".into(),
            accent_color: "#38bdf8".into(),
            high_contrast_labels: true,
            crisp_labels: false,
            label_glow: false,
            font_size: 22,
            post_hint: PostHint::Normal,
            exit_on_click: true,
            move_step: 8,
            hold_move_step: 24,
            smooth_movement: false,
            scroll_step: 120,
            span_all_monitors: false,
            keys: Keys::default(),
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            left: "h".into(),
            down: "j".into(),
            up: "k".into(),
            right: "l".into(),
            left_click: "m".into(),
            middle_click: ",".into(),
            right_click: ".".into(),
            drag: "v".into(),
            scroll_up: "e".into(),
            scroll_down: "d".into(),
            subdivide: "space".into(),
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .context("could not determine the config directory")?
            .join("kbmouse")
            .join("config.toml"))
    }

    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            let body = format!(
                "# kbmouse configuration. GUI saves apply live; restart after manual edits.\n{}",
                toml::to_string_pretty(&Self::default())?
            );
            fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let body = format!(
            "# kbmouse configuration. GUI saves apply live; restart after manual edits.\n{}",
            toml::to_string_pretty(self)?
        );
        fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn validate(&self) -> Result<()> {
        if self.label_style == LabelStyle::Sequences {
            let characters: Vec<_> = self.alphabet.chars().collect();
            let unique: std::collections::HashSet<_> = characters.iter().collect();
            if unique.len() < 2 {
                anyhow::bail!("alphabet must contain at least two characters");
            }
            if unique.len() != characters.len() {
                anyhow::bail!("alphabet may not contain duplicate characters");
            }
        }
        if self.target_cell_px == 0 || self.grid_rows == Some(0) || self.grid_cols == Some(0) {
            anyhow::bail!("grid dimensions and target_cell_px must be greater than zero");
        }
        if self.move_step <= 0 || self.hold_move_step <= 0 || self.scroll_step <= 0 {
            anyhow::bail!("movement and scroll speeds must be greater than zero");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_uses_defaults() {
        let config: Config = toml::from_str("leader = \"f9\"\n").unwrap();
        assert_eq!(config.leader, "f9");
        assert_eq!(config.keys.left, "h");
        assert_eq!(config.post_hint, PostHint::Normal);
    }

    #[test]
    fn invalid_grid_is_rejected() {
        let config: Config = toml::from_str("target_cell_px = 0").unwrap();
        assert!(config.validate().is_err());
    }
}
