mod accounts;
#[cfg(feature = "embedded")]
mod embed;
mod logos;
mod theme;
mod themes;
mod window;

use std::cell::RefCell;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use gtk::prelude::*;
use gtk::{gio, glib};
use tuner_app::Config;
use tuner_mpv::MpvOptions;

pub const APP_ID: &str = "io.github.tuner_iptv.Tuner";

/// Lean, Linux-native IPTV player written in Rust, with a native GTK4 interface.
/// Browses M3U playlists and Xtream Codes accounts and plays channels in mpv.
#[derive(Debug, Parser)]
#[command(name = "tuner-gtk", version, about)]
struct Cli {
    /// Name of a source from the config file, or a playlist path / http(s)
    /// URL. Defaults to the first configured source.
    source: Option<String>,

    /// Config file [default: ~/.config/tuner/config.toml].
    #[arg(long)]
    config: Option<PathBuf>,

    /// mpv executable [default: from config, else "mpv"].
    #[arg(long)]
    mpv: Option<String>,

    /// Extra argument passed to mpv, repeatable (e.g. --mpv-arg=--hwdec=auto).
    /// Added after the config's mpv args.
    #[arg(long = "mpv-arg", allow_hyphen_values = true)]
    mpv_args: Vec<String>,

    /// Colour theme: a built-in name (see --list-themes), or "custom" for
    /// theme.toml next to the config [default: `theme` in the config, else
    /// "custom"].
    #[arg(long)]
    theme: Option<String>,

    /// Print the available themes and exit.
    #[arg(long)]
    list_themes: bool,
}

fn main() -> Result<glib::ExitCode> {
    let cli = Cli::parse();
    if cli.list_themes {
        println!("custom   theme.toml next to config.toml (default; Tokyo Night without it)");
        for theme in themes::ALL {
            let kind = if theme.dark { "dark" } else { "light" };
            println!("{:<22} {} ({kind})", theme.name, theme.label);
        }
        return Ok(glib::ExitCode::SUCCESS);
    }

    // A broken config file is reported before any window opens. With no
    // accounts and no argument the window opens on "Add account" instead of
    // failing like the TUI.
    let config_path = cli.config.or_else(Config::default_path);
    let config = match &config_path {
        Some(path) => Config::load(path)?,
        None => Config::default(),
    };
    // An unknown theme name fails like a broken config, before any window.
    let theme = theme::Choice::parse(cli.theme.as_deref().or(config.theme.as_deref()))?;
    let initial = match cli.source.as_deref() {
        None if config.sources.is_empty() => None,
        arg => {
            let resolved = config.resolve(arg)?;
            let name = match arg {
                Some(arg) => config.source(arg).map(|c| c.name.clone()),
                None => config.sources.first().map(|c| c.name.clone()),
            };
            Some((name, resolved))
        }
    };
    let mpv = MpvOptions {
        program: cli
            .mpv
            .or_else(|| config.mpv.program.clone())
            .unwrap_or_else(|| "mpv".into()),
        extra_args: config
            .mpv
            .args
            .iter()
            .cloned()
            .chain(cli.mpv_args)
            .collect(),
    };

    // NON_UNIQUE: each launch is its own window/source; the stream guard,
    // not D-Bus uniqueness, prevents exceeding the account's limit.
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let setup = RefCell::new(Some(window::Start {
        config,
        config_path,
        initial,
        mpv,
        theme,
    }));
    app.connect_activate(move |app| {
        if let Some(start) = setup.borrow_mut().take() {
            window::build(app, start);
        }
    });
    // Our arguments were already parsed by clap; don't let GTK parse them.
    Ok(app.run_with_args::<&str>(&[]))
}
