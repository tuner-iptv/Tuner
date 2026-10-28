//! Theming. By default we use the user's theme file, `theme.toml` next to
//! `config.toml`: a palette of named hex colours that we turn into GTK CSS
//! and reload when it changes. Without that file, or when the user picks one
//! by name, a built-in palette from [`crate::themes`] is used. See
//! `docs/gui.md`.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Result, bail};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::themes::{self, Builtin};

/// Colours used by the stylesheet. Values are validated `#rrggbb`-style hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub background: String,
    pub dark_background: String,
    pub darker_background: String,
    pub lighter_background: String,
    pub foreground: String,
    pub dark_foreground: String,
    pub bright_foreground: String,
    pub accent: String,
    pub selection: String,
    pub muted: String,
    pub warning: String,
    pub error: String,
}

impl Default for Palette {
    /// Tokyo Night.
    fn default() -> Self {
        themes::DEFAULT.palette()
    }
}

/// Where the palette comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Choice {
    /// The user's theme file, reloaded when it changes; Tokyo Night when
    /// there is none.
    #[default]
    Custom,
    /// A built-in palette that never changes.
    Builtin(&'static Builtin),
}

impl Choice {
    /// The `theme` setting or `--theme` value: a built-in name, or `custom`.
    /// `None` is `custom`.
    pub fn parse(name: Option<&str>) -> Result<Self> {
        let Some(name) = name else {
            return Ok(Self::Custom);
        };
        if name.trim().eq_ignore_ascii_case("custom") {
            return Ok(Self::Custom);
        }
        match themes::find(name) {
            Some(theme) => Ok(Self::Builtin(theme)),
            None => bail!(
                "unknown theme '{name}'; available: custom, {}\n\
                 see `tuner-gtk --list-themes`",
                themes::ALL
                    .iter()
                    .map(|t| t.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl Palette {
    /// Parse a theme file. Missing or invalid keys fall back to related
    /// colours, then to the default palette.
    pub fn from_toml(text: &str) -> Option<Self> {
        let table: toml::Table = toml::from_str(text).ok()?;
        let get = |key: &str| {
            table
                .get(key)
                .and_then(toml::Value::as_str)
                .filter(|v| is_hex_color(v))
                .map(str::to_string)
        };
        let base = Self::default();
        let background = get("background")?;
        let foreground = get("foreground")?;
        Some(Self {
            dark: table.get("mode").and_then(toml::Value::as_str) != Some("light"),
            dark_background: get("dark_background").unwrap_or_else(|| background.clone()),
            darker_background: get("darker_background")
                .or_else(|| get("dark_background"))
                .unwrap_or_else(|| background.clone()),
            lighter_background: get("lighter_background")
                .or_else(|| get("selection"))
                .unwrap_or_else(|| background.clone()),
            dark_foreground: get("dark_foreground")
                .or_else(|| get("muted"))
                .unwrap_or_else(|| foreground.clone()),
            bright_foreground: get("bright_foreground").unwrap_or_else(|| foreground.clone()),
            accent: get("accent").or_else(|| get("blue")).unwrap_or(base.accent),
            selection: get("selection")
                .or_else(|| get("lighter_background"))
                .unwrap_or(base.selection),
            muted: get("muted").unwrap_or(base.muted),
            warning: get("yellow").unwrap_or(base.warning),
            error: get("red").unwrap_or(base.error),
            background,
            foreground,
        })
    }

    pub fn to_css(&self) -> String {
        let Palette {
            background: bg,
            dark_background: dark_bg,
            darker_background: darker_bg,
            lighter_background: light_bg,
            foreground: fg,
            bright_foreground: bright,
            accent,
            selection,
            muted,
            warning,
            error,
            ..
        } = self;
        format!(
            r#"
window, window.background, dialog {{ background-color: {bg}; color: {fg}; }}
headerbar {{ background: {dark_bg}; color: {fg}; box-shadow: none; border-bottom: 1px solid {muted}; }}
headerbar button, headerbar togglebutton {{ color: {fg}; }}
.browser-header {{ padding: 6px 12px; min-height: 40px; }}
.brand {{ font-size: 1.2em; font-weight: bold; color: {bright}; margin-right: 12px; }}
.browser-header searchentry {{ min-height: 28px; }}
.browser-header button, .browser-header togglebutton {{ border-radius: 8px; min-width: 32px; min-height: 32px; padding: 0 6px; }}
.browser-header windowcontrols {{ margin-left: 6px; }}
.browser-header windowcontrols button {{ min-width: 32px; min-height: 32px; padding: 0; margin: 0; }}
.browser-header windowcontrols button image {{ -gtk-icon-size: 16px; }}
.account-menu popover contents {{ background: {dark_bg}; color: {fg}; border: 1px solid {muted}; border-radius: 10px; padding: 8px; min-width: 260px; }}
.account-menu popover .section-label {{ margin: 4px 10px 8px; letter-spacing: 1px; }}
.account-menu popover button.flat {{ background: transparent; color: {fg}; padding: 8px 10px; border-radius: 6px; }}
.account-menu popover button.flat:hover {{ background: {light_bg}; }}
.account-menu popover button.account-row.current {{ background: {selection}; color: {accent}; }}
.account-menu popover separator {{ margin: 6px 0; }}
.account-menu > button {{ padding: 0 12px; }}
.account-menu .account-name {{ font-weight: 600; color: {bright}; }}
.account-menu > button .secondary-label {{ font-size: 0.9em; }}
.account-menu popover label {{ font-size: 0.9em; }}
.account-menu popover .secondary-label {{ font-size: 0.8em; }}
.account-menu popover .section-label {{ font-size: 0.75em; }}
.theme-picker popover contents {{ background: {dark_bg}; color: {fg}; border: 1px solid {muted}; border-radius: 10px; padding: 6px; min-width: 240px; }}
.theme-picker popover list, .theme-picker popover scrolledwindow {{ background: {dark_bg}; }}
.theme-picker popover list > row {{ background: {dark_bg}; color: {fg}; padding: 8px 12px; border-radius: 6px; }}
.theme-picker popover list > row:hover {{ background: {light_bg}; }}
.theme-picker popover list > row:selected {{ background: {selection}; color: {bright}; }}
.theme-picker popover list > row:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.theme-picker popover list > row image {{ color: {accent}; }}
.sidebar, .sidebar list {{ background: {dark_bg}; color: {fg}; }}
.resize-handle {{ min-width: 6px; background: {dark_bg}; border: none; padding: 0; margin: 0; }}
.resize-handle:hover, .resize-handle:active, .resize-handle:focus-visible {{ background: {accent}; }}
.section-label {{ color: mix({fg}, {bg}, 0.15); font-size: 0.9em; font-weight: 600; }}
.sidebar-heading {{ margin: 24px 20px 14px; }}
.sidebar row {{ padding: 10px 12px; margin: 2px 10px; border-radius: 8px; }}
.sidebar row:hover {{ background: {light_bg}; }}
.sidebar row:selected {{ background: {selection}; color: {accent}; font-weight: 600; }}
.sidebar .count {{ color: mix({fg}, {bg}, 0.15); font-size: 0.9em; font-weight: normal; font-feature-settings: "tnum"; }}
.list-heading {{ padding: 24px 22px 14px; }}
.section-title {{ color: {bright}; font-size: 1.3em; font-weight: bold; }}
.secondary-label {{ color: mix({fg}, {bg}, 0.15); font-size: 0.95em; }}
listview, listview > row {{ background: {bg}; color: {fg}; }}
.channel-list {{ padding-bottom: 12px; }}
.channel-list > row {{ padding: 8px 12px; margin: 2px 10px; border: 1px solid transparent; border-radius: 8px; }}
.channel-list > row:hover {{ background: {light_bg}; }}
.channel-list > row:selected {{ background: {selection}; color: {bright}; border-color: {muted}; }}
.channel-list > row:focus-visible, .sidebar row:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.channel-row {{ min-height: 46px; }}
.view-controls button {{ padding: 6px 8px; min-width: 20px; min-height: 20px; }}
.section-switch button {{ padding: 6px 6px; min-width: 0; }}
.channel-cards {{ background: {bg}; color: {fg}; padding: 0 10px 12px; }}
.channel-cards > child {{ padding: 16px 12px 10px; margin: 5px; background: {dark_bg}; border: 1px solid {muted}; border-radius: 10px; }}
.channel-cards > child:hover {{ background: {light_bg}; }}
.channel-cards > child:selected {{ background: {selection}; border-color: {accent}; }}
.channel-cards > child:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
.channel-card .logo-tile {{ padding: 10px; border-radius: 12px; }}
.channel-card .logo-initials {{ font-size: 1.7em; }}
.channel-card .channel-name {{ color: {bright}; }}
.channel-card .playing {{ font-size: 0.85em; }}
.channel-name {{ font-weight: 600; }}
.channel-subtitle {{ color: mix({fg}, {bg}, 0.15); font-size: 0.9em; }}
.channel-number {{ color: mix({fg}, {bg}, 0.25); font-size: 0.9em; font-feature-settings: "tnum"; }}
.playing {{ color: {accent}; font-weight: bold; }}
.channel-row .playing {{ font-size: 0.85em; }}
.logo-tile {{ background: {light_bg}; color: mix({fg}, {bg}, 0.15); border: 1px solid alpha({muted}, 0.6); border-radius: 8px; padding: 5px; }}
.logo-initials {{ color: {bright}; font-weight: 600; }}
.logo-frame .logo-initials {{ font-size: 2.2em; }}
.details {{ background: {dark_bg}; }}
.details-title {{ font-size: 1.5em; font-weight: bold; color: {bright}; }}
.field-label {{ color: mix({fg}, {bg}, 0.15); }}
.url-value {{ font-family: monospace; font-size: 0.85em; }}
.logo-frame {{ background: {light_bg}; color: {fg}; border-radius: 12px; }}
.channel-play {{ padding: 11px 14px; border-radius: 8px; font-weight: 600; }}
entry, searchentry, text {{ background: {light_bg}; color: {fg}; caret-color: {accent}; }}
entry, searchentry {{ border: 1px solid {muted}; border-radius: 6px; box-shadow: none; }}
entry:focus-within, searchentry:focus-within {{ border-color: {accent}; outline: none; }}
button {{ background: {light_bg}; color: {fg}; border: 1px solid {muted}; box-shadow: none; }}
button:hover {{ background: {selection}; }}
button:checked {{ background: {selection}; border-color: {accent}; }}
button.suggested-action {{ background: {accent}; color: {bg}; border-color: {accent}; }}
button.suggested-action:hover {{ background: {bright}; }}
button.destructive-action {{ background: {error}; color: {bg}; border-color: {error}; }}
.player-bar {{ background: {dark_bg}; border-top: 1px solid {muted}; padding: 14px 20px; }}
.player-title {{ font-weight: bold; color: {bright}; }}
.player-bar .dim-label {{ font-size: 0.9em; }}
.playback-state {{ color: {accent}; font-size: 0.9em; margin: 0 12px; }}
.player-bar button.circular {{ border-radius: 999px; min-width: 38px; min-height: 38px; padding: 0; }}
.player-bar button.circular:hover {{ background: {accent}; color: {bg}; border-color: {accent}; }}
.statusbar {{ background: {darker_bg}; color: mix({fg}, {bg}, 0.15); padding: 7px 20px; font-size: 0.85em; }}
.statusbar .now-playing {{ color: {accent}; }}
.statusbar .update {{ color: {accent}; }}
.warning {{ color: {warning}; }}
.error {{ color: {error}; }}
.dim-label {{ color: mix({fg}, {bg}, 0.15); }}
scrollbar slider {{ background: {muted}; }}
scrollbar {{ background: transparent; }}
"#
        )
    }
}

/// The theme file that belongs to `config`: `theme.toml` in the same
/// directory.
pub fn file_beside(config: &Path) -> PathBuf {
    config.with_file_name("theme.toml")
}

/// The palette for `choice`. The theme file's if `file` exists and is
/// valid, else the default.
pub fn load(choice: Choice, file: Option<&Path>) -> Palette {
    match choice {
        Choice::Builtin(theme) => theme.palette(),
        Choice::Custom => file
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| Palette::from_toml(&text))
            .unwrap_or_default(),
    }
}

/// Directories to watch for changes to the theme file. Its own directory
/// catches edits and editors that replace the file. When it is a symlink,
/// the target's directory and that directory's parent catch tools that
/// rewrite the target or swap out the whole directory holding it.
fn watched_dirs(file: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = file.parent().map(Path::to_path_buf).into_iter().collect();
    if let Ok(target) = std::fs::read_link(file) {
        let target = match file.parent() {
            Some(dir) => dir.join(target),
            None => target,
        };
        if let Some(dir) = target.parent() {
            dirs.extend(dir.parent().map(Path::to_path_buf));
            dirs.push(dir.to_path_buf());
        }
    }
    dirs
}

/// The installed theme. Holds the file monitors that reload the CSS when the
/// theme file changes (while it is in use), and lets the user switch theme
/// at runtime. Drop it to stop following the theme file.
pub struct Live {
    provider: gtk::CssProvider,
    choice: Rc<Cell<Choice>>,
    file: Option<PathBuf>,
    _monitors: Vec<gio::FileMonitor>,
}

impl Live {
    pub fn choice(&self) -> Choice {
        self.choice.get()
    }

    /// Recolour the window now.
    pub fn set(&self, choice: Choice) {
        self.choice.set(choice);
        apply(&self.provider, choice, self.file.as_deref());
    }
}

/// Apply the palette for `choice` to `display`. While the choice is
/// [`Choice::Custom`], also reload when the theme `file` changes.
pub fn install(display: &gdk::Display, choice: Choice, file: Option<PathBuf>) -> Live {
    let provider = gtk::CssProvider::new();
    apply(&provider, choice, file.as_deref());
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // The monitors stay for the window's life and reload only while the
    // theme file is in use, so switching back to it works.
    let choice = Rc::new(Cell::new(choice));
    let mut monitors = Vec::new();
    for watched in file.as_deref().map(watched_dirs).unwrap_or_default() {
        let monitor = gio::File::for_path(&watched)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE);
        if let Ok(monitor) = monitor {
            let (provider, choice, file) = (provider.clone(), choice.clone(), file.clone());
            monitor.connect_changed(move |_, _, _, _| {
                if choice.get() == Choice::Custom {
                    apply(&provider, Choice::Custom, file.as_deref());
                }
            });
            monitors.push(monitor);
        }
    }
    Live {
        provider,
        choice,
        file,
        _monitors: monitors,
    }
}

fn apply(provider: &gtk::CssProvider, choice: Choice, file: Option<&Path>) {
    let palette = load(choice, file);
    provider.load_from_data(&palette.to_css());
    if let Some(settings) = gtk::Settings::default() {
        // GTK ≥ 4.20 replaced `gtk-application-prefer-dark-theme`.
        let scheme = if palette.dark { "dark" } else { "light" };
        let value = settings
            .find_property("gtk-interface-color-scheme")
            .and_then(|pspec| glib::EnumClass::with_type(pspec.value_type()))
            .and_then(|class| class.to_value_by_nick(scheme));
        if let Some(value) = value {
            settings.set_property_from_value("gtk-interface-color-scheme", &value);
        } else {
            settings.set_gtk_application_prefer_dark_theme(palette.dark);
        }
    }
}

/// Only plain hex colours reach the stylesheet, so a theme file can never
/// inject CSS.
pub(crate) fn is_hex_color(v: &str) -> bool {
    v.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_file() {
        let text = r##"
            mode = "light"
            accent = "#ff0000"
            background = "#ffffff"
            foreground = "#000000"
            selection = "#eeeeee"
            yellow = "#ffcc00"
        "##;
        let p = Palette::from_toml(text).unwrap();
        assert!(!p.dark);
        assert_eq!(p.accent, "#ff0000");
        assert_eq!(p.dark_background, "#ffffff");
        assert_eq!(p.lighter_background, "#eeeeee");
        assert_eq!(p.dark_foreground, "#000000");
        assert_eq!(p.warning, "#ffcc00");
    }

    #[test]
    fn requires_background_and_foreground() {
        assert_eq!(Palette::from_toml(r##"accent = "#fff""##), None);
        assert_eq!(Palette::from_toml("not toml ["), None);
    }

    #[test]
    fn rejects_non_hex_values() {
        let text = r##"
            background = "#000000"
            foreground = "#ffffff"
            accent = "red; } * { color: red"
        "##;
        let p = Palette::from_toml(text).unwrap();
        assert_eq!(p.accent, Palette::default().accent);
        assert!(!p.to_css().contains("red;"));
    }

    #[test]
    fn choice_parses_names() {
        assert_eq!(Choice::parse(None).unwrap(), Choice::Custom);
        assert_eq!(Choice::parse(Some("Custom")).unwrap(), Choice::Custom);
        assert_eq!(
            Choice::parse(Some("catppuccin_mocha")).unwrap(),
            Choice::Builtin(themes::find("catppuccin-mocha").unwrap())
        );
        let err = Choice::parse(Some("nope")).unwrap_err().to_string();
        assert!(err.contains("unknown theme 'nope'"), "{err}");
        assert!(err.contains("catppuccin-latte"), "{err}");
    }

    #[test]
    fn a_builtin_choice_ignores_the_theme_file() {
        let latte = Choice::parse(Some("catppuccin-latte")).unwrap();
        let p = load(latte, Some(Path::new("/nonexistent/theme.toml")));
        assert!(!p.dark);
        assert_eq!(p.background, "#eff1f5");
    }

    #[test]
    fn custom_without_a_file_is_the_default() {
        let missing = Path::new("/nonexistent/theme.toml");
        assert_eq!(load(Choice::Custom, Some(missing)), Palette::default());
        assert_eq!(load(Choice::Custom, None), Palette::default());
    }

    #[test]
    fn theme_file_sits_beside_the_config() {
        assert_eq!(
            file_beside(Path::new("/home/u/.config/tuner/config.toml")),
            Path::new("/home/u/.config/tuner/theme.toml")
        );
    }

    #[test]
    fn watches_a_symlinked_theme_files_target() {
        let dir = std::env::temp_dir().join(format!("tuner-theme-{}", std::process::id()));
        let target = dir.join("elsewhere").join("current");
        std::fs::create_dir_all(&target).unwrap();
        let file = dir.join("theme.toml");
        let _ = std::fs::remove_file(&file);
        assert_eq!(watched_dirs(&file), std::slice::from_ref(&dir));
        std::os::unix::fs::symlink(target.join("colors.toml"), &file).unwrap();
        assert_eq!(
            watched_dirs(&file),
            [dir.clone(), dir.join("elsewhere"), target]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_builtin_makes_css() {
        for t in themes::ALL {
            let css = t.palette().to_css();
            assert!(css.contains(t.background), "{}", t.name);
            assert!(css.contains(t.accent), "{}", t.name);
        }
    }

    #[test]
    fn css_uses_palette() {
        let css = Palette::default().to_css();
        assert!(css.contains("background-color: #1a1b26"));
        assert!(css.contains(".playing { color: #7aa2f7"));
    }
}
