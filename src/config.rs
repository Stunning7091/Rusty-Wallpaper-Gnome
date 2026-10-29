use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gtk::glib;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Wallpaper for monitors that have no `[monitor.<connector>]` entry.
    #[serde(default)]
    pub default: Option<Wallpaper>,
    /// Per-monitor wallpapers, keyed by connector name (e.g. `HDMI-1`).
    #[serde(default)]
    pub monitor: BTreeMap<String, Wallpaper>,
    #[serde(default)]
    pub pause: Pause,
    /// Prefer VA-API decoders over software decoding.
    #[serde(default = "enabled")]
    pub hardware_decoding: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wallpaper {
    pub file: PathBuf,
    #[serde(default)]
    pub fit: Fit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Fill the monitor, cropping the video if aspect ratios differ.
    #[default]
    Cover,
    /// Show the whole video, letterboxed with black bars.
    Contain,
    /// Stretch the video to the monitor.
    Fill,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pause {
    /// Pause a monitor's wallpaper while a fullscreen window covers it.
    #[serde(default = "enabled")]
    pub fullscreen: bool,
}

impl Default for Pause {
    fn default() -> Self {
        Self { fullscreen: true }
    }
}

fn enabled() -> bool {
    true
}

impl Config {
    pub fn default_path() -> PathBuf {
        glib::user_config_dir()
            .join("rusty-wallpaper")
            .join("config.toml")
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut config: Config =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;

        let base = path.parent().unwrap_or(Path::new("."));
        for wallpaper in config.default.iter_mut().chain(config.monitor.values_mut()) {
            wallpaper.file = expand_path(&wallpaper.file, base);
        }
        Ok(config)
    }

    pub fn wallpaper_for(&self, connector: &str) -> Option<&Wallpaper> {
        self.monitor.get(connector).or(self.default.as_ref())
    }
}

/// Expands a leading `~` and resolves relative paths against the config file's directory.
fn expand_path(path: &Path, base: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~") {
        glib::home_dir().join(rest)
    } else if path.is_relative() {
        base.join(path)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_per_monitor_config() {
        let config: Config = toml::from_str(
            r#"
            [default]
            file = "default.mp4"

            [monitor.HDMI-1]
            file = "/videos/ocean.mp4"
            fit = "contain"

            [pause]
            fullscreen = false
            "#,
        )
        .unwrap();

        assert_eq!(config.wallpaper_for("HDMI-1").unwrap().fit, Fit::Contain);
        assert_eq!(
            config.wallpaper_for("HDMI-2").unwrap().file,
            PathBuf::from("default.mp4")
        );
        assert!(!config.pause.fullscreen);
        assert!(config.hardware_decoding);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<Config>("[monitor.HDMI-1]\nfiel = \"x.mp4\"").is_err());
    }

    #[test]
    fn expands_paths() {
        let base = Path::new("/etc/rw");
        assert_eq!(
            expand_path(Path::new("a.mp4"), base),
            PathBuf::from("/etc/rw/a.mp4")
        );
        assert_eq!(
            expand_path(Path::new("/v/a.mp4"), base),
            PathBuf::from("/v/a.mp4")
        );
        assert_eq!(
            expand_path(Path::new("~/v/a.mp4"), base),
            glib::home_dir().join("v/a.mp4")
        );
    }
}
