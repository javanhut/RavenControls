//! Raven's glass shell, in lockstep with Settings, Store and Power.
//!
//! The palette below is the same one in `RavenSettingsUI/src/ui/theme.rs` and
//! `RavenBatteryManagement/src/style.css`. It is expressed as libadwaita's
//! named colours so that every stock widget -- switches, dropdowns, scales, the
//! header bar -- follows without being styled one at a time.
//!
//! "Glass" is alpha only. The blur behind the window is Huginn's to draw; this
//! side just declines to be opaque. The class is on the window rather than
//! compiled in, so `transparency = false` in `desktop.toml` turns it off
//! without a second stylesheet.
//!
//! Two providers, at two priorities: the base palette, and an override that
//! carries the accent and the light-mode swap. Rebuilding only the second is
//! what lets the accent change without reloading everything.

use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::config::{self, Appearance, ThemeMode};

pub const BASE_CSS: &str = r#"
/* Raven dark palette. Keep in step with Settings, Store and Power. */
@define-color window_bg_color #16161f;
@define-color window_fg_color #d0d0e0;
@define-color headerbar_bg_color #16161f;
@define-color headerbar_fg_color #d0d0e0;
@define-color headerbar_border_color #2a2a3a;
@define-color headerbar_shade_color rgba(0,0,0,0.36);
@define-color view_bg_color #1a1a26;
@define-color view_fg_color #d0d0e0;
@define-color card_bg_color #1e1e2b;
@define-color card_fg_color #d0d0e0;
@define-color dialog_bg_color #1e1e2b;
@define-color dialog_fg_color #d0d0e0;
@define-color popover_bg_color #1e1e2b;
@define-color popover_fg_color #d0d0e0;
@define-color sidebar_bg_color #141420;
@define-color borders #2a2a3a;

window.raven { background-color: @window_bg_color; }
headerbar {
  box-shadow: none;
  border-bottom: 1px solid #2a2a3a;
  background-color: transparent;
}
toolbarview, stack { background-color: transparent; }

/* The compositor provides the blur behind these translucent surfaces. */
window.raven.glass { background-color: alpha(#16161f, 0.72); }
window.raven.glass .sidebar {
  background-color: alpha(#0e0e16, 0.45);
  border-right-color: alpha(#ffffff, 0.07);
}
window.raven.glass .card,
window.raven.glass .raven-card {
  background-color: alpha(#ffffff, 0.085);
  border-color: alpha(#ffffff, 0.11);
}
window.raven.glass headerbar { border-bottom-color: alpha(#ffffff, 0.07); }
window.raven.glass list.boxed-list,
window.raven.glass list.boxed-list > row {
  background-color: alpha(#ffffff, 0.04);
}
window.raven.glass switch,
window.raven.glass scale trough,
window.raven.glass dropdown > button,
window.raven.glass spinbutton {
  background-color: alpha(#ffffff, 0.10);
}

.sidebar {
  background-color: #141420;
  border-right: 1px solid #2a2a3a;
  padding: 18px 14px;
}
.sidebar .brand { margin: 0 8px 14px 8px; }
.sidebar .brand image { color: @accent_bg_color; -gtk-icon-size: 34px; }
.sidebar .app-title { font-size: 19px; font-weight: 700; }
.sidebar .app-subtitle { font-size: 12px; color: alpha(@window_fg_color, 0.55); }
.sidebar list.navigation-sidebar { background: transparent; }
.sidebar list.navigation-sidebar row {
  border-radius: 10px;
  padding: 9px 10px;
  margin: 2px 0;
}
.sidebar list.navigation-sidebar row:selected {
  background-color: alpha(@accent_bg_color, 0.22);
  color: @window_fg_color;
  box-shadow: inset 0 0 0 1px alpha(@accent_bg_color, 0.55);
}
.sidebar list.navigation-sidebar row image { color: @accent_bg_color; }

.page-scroll > viewport { background-color: transparent; }
.page { padding: 28px 34px 42px; }
.card, .raven-card {
  background-color: #1e1e2b;
  border: 1px solid #2a2a3a;
  border-radius: 14px;
  padding: 16px;
}
.raven-card .card-title { font-weight: 600; font-size: 15px; }
.raven-card .dim, .dim-label { color: alpha(@window_fg_color, 0.60); }
.status-card { padding: 12px 14px; }
.status-card image { color: @accent_bg_color; }

.page-title { font-size: 24px; font-weight: 700; color: @window_fg_color; }
.page-subtitle { color: alpha(@window_fg_color, 0.6); }
.section-title, .card-title { font-size: 17px; font-weight: 700; }
.eyebrow { color: @accent_bg_color; font-size: 11px; font-weight: 750; }
.note { color: alpha(@window_fg_color, 0.55); font-size: 12px; }
.mono { font-family: monospace; }

/* The fan-curve graph draws itself; this is the frame around it. */
.curve-frame {
  border-radius: 12px;
  border: 1px solid @borders;
  background-color: alpha(#0e0e16, 0.35);
}
window.raven.glass .curve-frame {
  border-color: alpha(#ffffff, 0.11);
  background-color: alpha(#000000, 0.18);
}
"#;

/// The light-mode half of the palette, swapped in over the base.
const LIGHT_CSS: &str = r#"
@define-color window_bg_color #eef0f6;
@define-color window_fg_color #1a1b26;
@define-color headerbar_bg_color #eef0f6;
@define-color headerbar_fg_color #1a1b26;
@define-color headerbar_border_color #d0d3e0;
@define-color view_bg_color #f7f8fc;
@define-color view_fg_color #1a1b26;
@define-color card_bg_color #f7f8fc;
@define-color card_fg_color #1a1b26;
@define-color dialog_bg_color #f7f8fc;
@define-color dialog_fg_color #1a1b26;
@define-color popover_bg_color #f7f8fc;
@define-color popover_fg_color #1a1b26;
@define-color sidebar_bg_color #e6e8f0;
@define-color borders #d0d3e0;
headerbar { border-bottom-color: #d0d3e0; }
.sidebar { background-color: #e6e8f0; border-right-color: #d0d3e0; }
.card, .raven-card { background-color: #f7f8fc; border-color: #d0d3e0; }
.curve-frame { background-color: alpha(#ffffff, 0.6); border-color: #d0d3e0; }
window.raven.glass { background-color: alpha(#eef0f6, 0.80); }
window.raven.glass .sidebar { background-color: alpha(#ffffff, 0.35); }
window.raven.glass .card,
window.raven.glass .raven-card {
  background-color: alpha(#ffffff, 0.45);
  border-color: alpha(#000000, 0.08);
}
window.raven.glass list.boxed-list,
window.raven.glass list.boxed-list > row { background-color: alpha(#ffffff, 0.40); }
window.raven.glass .curve-frame {
  background-color: alpha(#ffffff, 0.45);
  border-color: alpha(#000000, 0.08);
}
"#;

pub fn load_base() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(BASE_CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

thread_local! {
    static OVERRIDE: std::cell::RefCell<Option<gtk::CssProvider>> =
        const { std::cell::RefCell::new(None) };
}

/// Point every `@accent_bg_color` at the configured accent, set light or dark,
/// and put the window in or out of glass.
pub fn apply(window: &impl IsA<gtk::Widget>, appearance: &Appearance) {
    if appearance.transparency {
        window.add_css_class("glass");
    } else {
        window.remove_css_class("glass");
    }

    adw::StyleManager::default().set_color_scheme(match appearance.theme_mode {
        ThemeMode::Dark => adw::ColorScheme::ForceDark,
        ThemeMode::Light => adw::ColorScheme::ForceLight,
        // Raven is a dark desktop; "auto" prefers dark rather than asking a
        // portal that may not be running.
        ThemeMode::Auto => adw::ColorScheme::PreferDark,
    });

    // Never interpolate an unvalidated string into a stylesheet.
    let accent = if config::is_hex(&appearance.accent) {
        appearance.accent.as_str()
    } else {
        config::DEFAULT_ACCENT
    };
    let css = format!(
        "@define-color accent_bg_color {accent};\n\
         @define-color accent_color {accent};\n{}",
        if appearance.theme_mode == ThemeMode::Light {
            LIGHT_CSS
        } else {
            ""
        }
    );

    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    OVERRIDE.with(|slot| {
        if let Some(old) = slot.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        *slot.borrow_mut() = Some(provider);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_is_the_one_the_other_raven_applications_use() {
        // If one of these drifts, RavenControls stops matching Settings and
        // Power on the same desktop, which is the whole point of copying them.
        for colour in [
            "@define-color window_bg_color #16161f;",
            "@define-color sidebar_bg_color #141420;",
            "@define-color borders #2a2a3a;",
            "window.raven.glass { background-color: alpha(#16161f, 0.72); }",
        ] {
            assert!(BASE_CSS.contains(colour), "missing: {colour}");
        }
    }

    /// Load a stylesheet into a real `CssProvider` and fail on any parse error.
    ///
    /// GTK does not reject a bad stylesheet -- it logs the rule it could not
    /// understand, drops it, and carries on looking almost right. A misspelled
    /// property or an unbalanced brace here would mean the glass silently does
    /// not apply on somebody else's machine, which is exactly the class of bug
    /// nobody notices until a screenshot looks wrong.
    fn assert_parses(name: &str, css: &str) {
        if gtk::init().is_err() {
            // No display: this runs under `cargo test` on a build machine.
            eprintln!("skipping the {name} CSS parse check: no display");
            return;
        }
        let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let provider = gtk::CssProvider::new();
        {
            let errors = errors.clone();
            provider.connect_parsing_error(move |_, section, error| {
                errors
                    .borrow_mut()
                    .push(format!("{}: {error}", section.to_str()));
            });
        }
        provider.load_from_string(css);
        let errors = errors.borrow();
        assert!(
            errors.is_empty(),
            "{name} does not parse:\n{}",
            errors.join("\n")
        );
    }

    #[test]
    fn the_base_stylesheet_parses() {
        assert_parses("BASE_CSS", BASE_CSS);
    }

    #[test]
    fn the_light_override_parses_on_top_of_the_base() {
        // Light mode is the half nobody developing on a dark desktop looks at.
        assert_parses("LIGHT_CSS", &format!("{BASE_CSS}\n{LIGHT_CSS}"));
    }

    #[test]
    fn the_accent_override_parses_with_a_real_accent_in_it() {
        assert_parses(
            "accent override",
            &format!(
                "@define-color accent_bg_color {};\n@define-color accent_color {};\n{LIGHT_CSS}",
                config::DEFAULT_ACCENT,
                config::DEFAULT_ACCENT
            ),
        );
    }

    #[test]
    fn glass_is_alpha_only_and_never_asks_gtk_to_blur() {
        // The blur belongs to the compositor. A `filter: blur()` here would be
        // drawn by GTK over the window's own contents, which looks wrong and
        // costs a full-window redraw every frame.
        assert!(!BASE_CSS.contains("blur("));
        assert!(!LIGHT_CSS.contains("blur("));
    }
}
