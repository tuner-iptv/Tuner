//! Built-in colour schemes, for use without a theme file or to override it
//! (`theme = "…"` in `config.toml`, or `--theme`). See `docs/gui.md`.
//!
//! Values come from each scheme's published palette. Where a scheme has no
//! shade darker (or, for light schemes, lighter) than its background, the
//! missing sidebar, status bar or hover shade is a blend of the background
//! with black or white. Each row maps onto [`Palette`]'s fields in order:
//! `lighter_background` is the hover/entry/button fill, `selection` the
//! selected row, `muted` borders and scrollbars.

use crate::theme::Palette;

/// A named, fixed palette.
#[derive(Debug, PartialEq, Eq)]
pub struct Builtin {
    /// Lowercase, hyphenated: what `theme = "…"` takes.
    pub name: &'static str,
    /// For humans, e.g. in `--list-themes`.
    pub label: &'static str,
    pub dark: bool,
    pub background: &'static str,
    pub dark_background: &'static str,
    pub darker_background: &'static str,
    pub lighter_background: &'static str,
    pub foreground: &'static str,
    pub dark_foreground: &'static str,
    pub bright_foreground: &'static str,
    pub accent: &'static str,
    pub selection: &'static str,
    pub muted: &'static str,
    pub warning: &'static str,
    pub error: &'static str,
}

impl Builtin {
    pub fn palette(&self) -> Palette {
        Palette {
            dark: self.dark,
            background: self.background.into(),
            dark_background: self.dark_background.into(),
            darker_background: self.darker_background.into(),
            lighter_background: self.lighter_background.into(),
            foreground: self.foreground.into(),
            dark_foreground: self.dark_foreground.into(),
            bright_foreground: self.bright_foreground.into(),
            accent: self.accent.into(),
            selection: self.selection.into(),
            muted: self.muted.into(),
            warning: self.warning.into(),
            error: self.error.into(),
        }
    }
}

/// What is used when there is no theme file.
pub const DEFAULT: &Builtin = &ALL[0];

/// Look a theme up by name, ignoring case and treating `_` and spaces like `-`.
pub fn find(name: &str) -> Option<&'static Builtin> {
    let wanted = name.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    ALL.iter().find(|t| t.name == wanted)
}

pub const ALL: &[Builtin] = &[
    Builtin {
        name: "tokyo-night",
        label: "Tokyo Night",
        dark: true,
        background: "#1a1b26",
        dark_background: "#13141c",
        darker_background: "#0e0e14",
        lighter_background: "#24283b",
        foreground: "#a9b1d6",
        dark_foreground: "#565f89",
        bright_foreground: "#c0caf5",
        accent: "#7aa2f7",
        selection: "#292e42",
        muted: "#414868",
        warning: "#e0af68",
        error: "#f7768e",
    },
    Builtin {
        name: "tokyo-night-storm",
        label: "Tokyo Night Storm",
        dark: true,
        background: "#24283b",
        dark_background: "#1f2335",
        darker_background: "#16161e",
        lighter_background: "#292e42",
        foreground: "#a9b1d6",
        dark_foreground: "#565f89",
        bright_foreground: "#c0caf5",
        accent: "#7aa2f7",
        selection: "#2e3c64",
        muted: "#414868",
        warning: "#e0af68",
        error: "#f7768e",
    },
    Builtin {
        name: "tokyo-night-moon",
        label: "Tokyo Night Moon",
        dark: true,
        background: "#222436",
        dark_background: "#1e2030",
        darker_background: "#191b29",
        lighter_background: "#2f334d",
        foreground: "#c8d3f5",
        dark_foreground: "#636da6",
        bright_foreground: "#c8d3f5",
        accent: "#82aaff",
        selection: "#2d3f76",
        muted: "#444a73",
        warning: "#ffc777",
        error: "#ff757f",
    },
    Builtin {
        name: "tokyo-night-day",
        label: "Tokyo Night Day",
        dark: false,
        background: "#e1e2e7",
        dark_background: "#d0d5e3",
        darker_background: "#c4c8da",
        lighter_background: "#f2f2f4",
        foreground: "#3760bf",
        dark_foreground: "#6172b0",
        bright_foreground: "#343b58",
        accent: "#2e7de9",
        selection: "#c4c8da",
        muted: "#a1a6c5",
        warning: "#8c6c3e",
        error: "#f52a65",
    },
    Builtin {
        name: "catppuccin-latte",
        label: "Catppuccin Latte",
        dark: false,
        background: "#eff1f5",
        dark_background: "#e6e9ef",
        darker_background: "#dce0e8",
        lighter_background: "#f9f9fb",
        foreground: "#5c5f77",
        dark_foreground: "#8c8fa1",
        bright_foreground: "#4c4f69",
        accent: "#8839ef",
        selection: "#ccd0da",
        muted: "#acb0be",
        warning: "#df8e1d",
        error: "#d20f39",
    },
    Builtin {
        name: "catppuccin-frappe",
        label: "Catppuccin Frappé",
        dark: true,
        background: "#303446",
        dark_background: "#292c3c",
        darker_background: "#232634",
        lighter_background: "#414559",
        foreground: "#b5bfe2",
        dark_foreground: "#838ba7",
        bright_foreground: "#c6d0f5",
        accent: "#ca9ee6",
        selection: "#51576d",
        muted: "#626880",
        warning: "#e5c890",
        error: "#e78284",
    },
    Builtin {
        name: "catppuccin-macchiato",
        label: "Catppuccin Macchiato",
        dark: true,
        background: "#24273a",
        dark_background: "#1e2030",
        darker_background: "#181926",
        lighter_background: "#363a4f",
        foreground: "#b8c0e0",
        dark_foreground: "#8087a2",
        bright_foreground: "#cad3f5",
        accent: "#c6a0f6",
        selection: "#494d64",
        muted: "#5b6078",
        warning: "#eed49f",
        error: "#ed8796",
    },
    Builtin {
        name: "catppuccin-mocha",
        label: "Catppuccin Mocha",
        dark: true,
        background: "#1e1e2e",
        dark_background: "#181825",
        darker_background: "#11111b",
        lighter_background: "#313244",
        foreground: "#bac2de",
        dark_foreground: "#7f849c",
        bright_foreground: "#cdd6f4",
        accent: "#cba6f7",
        selection: "#45475a",
        muted: "#585b70",
        warning: "#f9e2af",
        error: "#f38ba8",
    },
    Builtin {
        name: "gruvbox-dark",
        label: "Gruvbox Dark",
        dark: true,
        background: "#282828",
        dark_background: "#1d2021",
        darker_background: "#161819",
        lighter_background: "#3c3836",
        foreground: "#ebdbb2",
        dark_foreground: "#a89984",
        bright_foreground: "#fbf1c7",
        accent: "#83a598",
        selection: "#504945",
        muted: "#665c54",
        warning: "#fabd2f",
        error: "#fb4934",
    },
    Builtin {
        name: "gruvbox-light",
        label: "Gruvbox Light",
        dark: false,
        background: "#fbf1c7",
        dark_background: "#f2e5bc",
        darker_background: "#ebdbb2",
        lighter_background: "#fdf9e6",
        foreground: "#3c3836",
        dark_foreground: "#7c6f64",
        bright_foreground: "#282828",
        accent: "#076678",
        selection: "#d5c4a1",
        muted: "#bdae93",
        warning: "#b57614",
        error: "#9d0006",
    },
    Builtin {
        name: "nord",
        label: "Nord",
        dark: true,
        background: "#2e3440",
        dark_background: "#282e38",
        darker_background: "#232831",
        lighter_background: "#3b4252",
        foreground: "#d8dee9",
        dark_foreground: "#616e88",
        bright_foreground: "#eceff4",
        accent: "#88c0d0",
        selection: "#434c5e",
        muted: "#4c566a",
        warning: "#ebcb8b",
        error: "#bf616a",
    },
    Builtin {
        name: "dracula",
        label: "Dracula",
        dark: true,
        background: "#282a36",
        dark_background: "#21222c",
        darker_background: "#191a21",
        lighter_background: "#343746",
        foreground: "#f8f8f2",
        dark_foreground: "#6272a4",
        bright_foreground: "#f8f8f2",
        accent: "#bd93f9",
        selection: "#44475a",
        muted: "#6272a4",
        warning: "#ffb86c",
        error: "#ff5555",
    },
    Builtin {
        name: "rose-pine",
        label: "Rosé Pine",
        dark: true,
        background: "#191724",
        dark_background: "#15141f",
        darker_background: "#121019",
        lighter_background: "#1f1d2e",
        foreground: "#e0def4",
        dark_foreground: "#6e6a86",
        bright_foreground: "#e0def4",
        accent: "#c4a7e7",
        selection: "#403d52",
        muted: "#524f67",
        warning: "#f6c177",
        error: "#eb6f92",
    },
    Builtin {
        name: "rose-pine-moon",
        label: "Rosé Pine Moon",
        dark: true,
        background: "#232136",
        dark_background: "#1e1c2e",
        darker_background: "#181726",
        lighter_background: "#2a273f",
        foreground: "#e0def4",
        dark_foreground: "#6e6a86",
        bright_foreground: "#e0def4",
        accent: "#c4a7e7",
        selection: "#44415a",
        muted: "#56526e",
        warning: "#f6c177",
        error: "#eb6f92",
    },
    Builtin {
        name: "rose-pine-dawn",
        label: "Rosé Pine Dawn",
        dark: false,
        background: "#faf4ed",
        dark_background: "#f2e9e1",
        darker_background: "#e6dcd5",
        lighter_background: "#fffaf3",
        foreground: "#575279",
        dark_foreground: "#9893a5",
        bright_foreground: "#575279",
        accent: "#907aa9",
        selection: "#dfdad9",
        muted: "#cecacd",
        warning: "#ea9d34",
        error: "#b4637a",
    },
    Builtin {
        name: "solarized-dark",
        label: "Solarized Dark",
        dark: true,
        background: "#002b36",
        dark_background: "#002630",
        darker_background: "#002028",
        lighter_background: "#073642",
        foreground: "#93a1a1",
        dark_foreground: "#586e75",
        bright_foreground: "#eee8d5",
        accent: "#268bd2",
        selection: "#1f4751",
        muted: "#586e75",
        warning: "#b58900",
        error: "#dc322f",
    },
    Builtin {
        name: "solarized-light",
        label: "Solarized Light",
        dark: false,
        background: "#fdf6e3",
        dark_background: "#eee8d5",
        darker_background: "#e0dac8",
        lighter_background: "#fefbf4",
        foreground: "#586e75",
        dark_foreground: "#93a1a1",
        bright_foreground: "#073642",
        accent: "#268bd2",
        selection: "#e4dec9",
        muted: "#93a1a1",
        warning: "#b58900",
        error: "#dc322f",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::is_hex_color;

    fn channel(c: u8) -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(hex: &str) -> f64 {
        let v = |i: usize| channel(u8::from_str_radix(&hex[i..i + 2], 16).unwrap());
        0.2126 * v(1) + 0.7152 * v(3) + 0.0722 * v(5)
    }

    /// WCAG contrast ratio between two `#rrggbb` colours.
    fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn names_are_unique_and_normalised() {
        let mut seen = std::collections::HashSet::new();
        for t in ALL {
            assert!(seen.insert(t.name), "duplicate theme {}", t.name);
            assert_eq!(t.name, t.name.to_ascii_lowercase());
            assert!(
                t.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{}",
                t.name
            );
            assert_ne!(t.name, "custom", "reserved for the theme file");
        }
    }

    #[test]
    fn colours_are_six_digit_hex() {
        for t in ALL {
            let p = t.palette();
            for (field, value) in [
                ("background", &p.background),
                ("dark_background", &p.dark_background),
                ("darker_background", &p.darker_background),
                ("lighter_background", &p.lighter_background),
                ("foreground", &p.foreground),
                ("dark_foreground", &p.dark_foreground),
                ("bright_foreground", &p.bright_foreground),
                ("accent", &p.accent),
                ("selection", &p.selection),
                ("muted", &p.muted),
                ("warning", &p.warning),
                ("error", &p.error),
            ] {
                assert!(
                    is_hex_color(value) && value.len() == 7,
                    "{}: {field} = {value}",
                    t.name
                );
            }
        }
    }

    #[test]
    fn text_is_readable() {
        for t in ALL {
            assert!(
                contrast(t.foreground, t.background) >= 4.5,
                "{}: text on background",
                t.name
            );
            assert!(
                contrast(t.bright_foreground, t.background) >= 4.5,
                "{}: bright text on background",
                t.name
            );
            assert!(
                contrast(t.bright_foreground, t.selection) >= 4.5,
                "{}: text on a selected row",
                t.name
            );
            // Accent text, and the background colour on accent-filled buttons.
            assert!(
                contrast(t.accent, t.background) >= 3.0,
                "{}: accent against background",
                t.name
            );
        }
    }

    #[test]
    fn dark_flag_matches_the_background() {
        for t in ALL {
            assert_eq!(t.dark, luminance(t.background) < 0.5, "{}", t.name);
        }
    }

    #[test]
    fn find_ignores_case_and_separators() {
        assert_eq!(find("Tokyo_Night").unwrap().name, "tokyo-night");
        assert_eq!(find(" catppuccin mocha ").unwrap().name, "catppuccin-mocha");
        assert!(find("catppuccin").is_none());
        assert!(find("custom").is_none());
    }

    #[test]
    fn default_is_tokyo_night() {
        assert_eq!(DEFAULT.name, "tokyo-night");
    }
}
