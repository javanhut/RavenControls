//! The desktop's appearance settings, as Raven Settings writes them.
//!
//! `~/.config/desktop.toml` is the shared file: Raven Settings' Appearance page
//! writes it, and every Raven application reads it so that changing the accent
//! in one place changes it everywhere. RavenControls only reads, and only the
//! three fields it can act on -- the rest of that file belongs to other
//! applications and is deliberately not modelled here, so a field added
//! elsewhere cannot break this parse.

use std::path::PathBuf;

use serde::Deserialize;

/// The compositor's compiled-in accent, and the first swatch in Settings.
pub const DEFAULT_ACCENT: &str = "#7AA2F7";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    #[default]
    Dark,
    /// Follow the system preference, which on Raven means dark.
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme_mode: ThemeMode,
    /// `#RRGGBB`.
    pub accent: String,
    /// The glass shell: a translucent window for the compositor to blur behind.
    pub transparency: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::default(),
            accent: DEFAULT_ACCENT.into(),
            transparency: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct DesktopConfig {
    appearance: Appearance,
}

pub fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("desktop.toml")
}

/// The appearance to draw with.
///
/// A missing file is the ordinary case on a fresh machine and gives the
/// defaults; an unparseable one gives the defaults too, with a warning. Neither
/// is worth refusing to open a window over.
pub fn appearance() -> Appearance {
    let path = path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Appearance::default();
    };
    match toml::from_str::<DesktopConfig>(&text) {
        Ok(config) => config.appearance,
        Err(e) => {
            tracing::warn!("{}: {e}; using the default appearance", path.display());
            Appearance::default()
        }
    }
}

/// `#RRGGBB`, and nothing else, because this string is interpolated straight
/// into a stylesheet.
pub fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_file_is_read_the_way_settings_writes_it() {
        let config: DesktopConfig = toml::from_str(
            "[appearance]\ntheme_mode = \"light\"\naccent = \"#F7768E\"\ntransparency = false\n",
        )
        .unwrap();
        assert_eq!(config.appearance.theme_mode, ThemeMode::Light);
        assert_eq!(config.appearance.accent, "#F7768E");
        assert!(!config.appearance.transparency);
    }

    #[test]
    fn fields_other_applications_own_do_not_break_the_parse() {
        // desktop.toml is shared. A key this application has never heard of --
        // and there are many -- must not cost it its theme.
        let config: DesktopConfig = toml::from_str(
            "[appearance]\naccent = \"#22C5DD\"\nscale = 1.25\nblur = true\n\
             [general]\nremember_app_usage = false\n[personalization]\nwallpaper = \"/x.png\"\n",
        )
        .unwrap();
        assert_eq!(config.appearance.accent, "#22C5DD");
        // And the fields that were absent still get their defaults.
        assert_eq!(config.appearance.theme_mode, ThemeMode::Dark);
        assert!(config.appearance.transparency);
    }

    #[test]
    fn an_empty_or_absent_file_gives_the_raven_defaults() {
        let config: DesktopConfig = toml::from_str("").unwrap();
        assert_eq!(config.appearance.accent, DEFAULT_ACCENT);
        assert_eq!(config.appearance.theme_mode, ThemeMode::Dark);
        assert!(config.appearance.transparency);
    }

    #[test]
    fn only_a_real_hex_colour_reaches_the_stylesheet() {
        assert!(is_hex("#7AA2F7"));
        assert!(is_hex("#000000"));
        assert!(!is_hex("7AA2F7"));
        assert!(!is_hex("#7AA2F"));
        // The one that matters: this string is interpolated into CSS.
        assert!(!is_hex("#fff; } * { background: red"));
    }
}
