mod app;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tuner_app::{Config, Resolved, Source, StreamGuard};
use tuner_core::Library;
use tuner_mpv::{MpvOptions, Player};

use crate::app::{App, CatalogDone, CatalogJob};

/// How often to check whether the user closed the mpv window.
const TICK: Duration = Duration::from_millis(500);
const LOADING_TICK: Duration = Duration::from_millis(100);
/// Redraw rate while a list transition runs.
const ANIMATION_TICK: Duration = Duration::from_millis(25);

type LoadedSource = (Library, StreamGuard, String, Source);

/// Lean, Linux-native IPTV player written in Rust, with a keyboard-first terminal interface.
/// Browses M3U playlists and Xtream Codes accounts and plays channels in mpv.
#[derive(Debug, Parser)]
#[command(version, about)]
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = match cli.config.or_else(Config::default_path) {
        Some(path) => Config::load(&path)?,
        None => Config::default(),
    };
    let resolved = config.resolve(cli.source.as_deref())?;
    eprintln!("Loading {}…", resolved.source.label());
    let (library, guard, status, catalog) =
        load_source(resolved).context("could not load channels")?;

    let player = Player::new(MpvOptions {
        program: cli
            .mpv
            .or(config.mpv.program.clone())
            .unwrap_or_else(|| "mpv".into()),
        extra_args: config
            .mpv
            .args
            .iter()
            .cloned()
            .chain(cli.mpv_args)
            .collect(),
    });
    let mut app = App::new(library, player, guard).with_catalog(catalog);
    app.status = status;
    let current = match cli.source.as_deref() {
        Some(name) => config.source(name),
        None => config.sources.first(),
    }
    .map(|source| source.name.clone());
    app.set_accounts(
        config.sources.iter().map(|s| s.name.clone()).collect(),
        current,
    );

    // SIGTERM, or SIGHUP when the terminal window is closed, would otherwise
    // kill tuner without dropping the player and leave mpv streaming,
    // still using one of the account's connections.
    let terminate = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGINT,
    ] {
        signal_hook::flag::register(signal, terminate.clone())?;
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &config, &terminate);
    ratatui::restore();
    // Dropping the app quits mpv and releases the stream slot.
    drop(app);
    result
}

fn load_source(resolved: Resolved) -> Result<LoadedSource> {
    let loaded = resolved.source.load()?;
    let catalog = resolved.source.clone();
    // Explicit config wins; else trust the provider's limit; else assume 1.
    let max_streams = resolved.max_streams.or(loaded.max_connections).unwrap_or(1);
    let guard = StreamGuard::new(resolved.source, max_streams, resolved.limit_mode);
    let plural = if max_streams == 1 {
        "stream"
    } else {
        "streams"
    };
    let status = format!("{} · limit {max_streams} {plural}", loaded.summary);
    Ok((Library::new(loaded.channels), guard, status, catalog))
}

/// Only source data crosses the channel; playback stays on the TUI thread.
struct AccountLoad {
    name: String,
    result: Receiver<Result<LoadedSource>>,
}

impl AccountLoad {
    fn start(config: &Config, name: String) -> Result<Self> {
        let source = config
            .source(&name)
            .context("account is no longer configured")?
            .clone();
        let (sender, result) = mpsc::channel();
        std::thread::Builder::new()
            .name("account-loader".into())
            .spawn(move || {
                // Password commands may block too, so resolve on the worker.
                let loaded = source.resolve().and_then(load_source);
                let _ = sender.send(loaded);
            })
            .context("could not start account loader")?;
        Ok(Self { name, result })
    }

    fn poll(&self) -> Option<Result<LoadedSource>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("account loader stopped unexpectedly")))
            }
        }
    }
}

fn finish_account_switch(app: &mut App, name: String, result: Result<LoadedSource>) {
    match result {
        Ok((library, guard, status, catalog)) => {
            app.switch_account(name, library, guard, catalog, status)
        }
        Err(e) => app.account_load_failed(format!("{e:#}")),
    }
}

/// Run a catalogue request off the UI thread, so the loading overlay keeps
/// animating while the panel answers.
fn spawn_catalog(job: CatalogJob) -> Result<Receiver<CatalogDone>> {
    let (sender, result) = mpsc::channel();
    std::thread::Builder::new()
        .name("catalog-loader".into())
        .spawn(move || {
            let _ = sender.send(job.run());
        })
        .context("could not start catalogue loader")?;
    Ok(result)
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    config: &Config,
    terminate: &AtomicBool,
) -> Result<()> {
    let mut loading: Option<AccountLoad> = None;
    let mut catalog_load: Option<Receiver<CatalogDone>> = None;
    while !app.should_quit() && !terminate.load(Ordering::Relaxed) {
        if let Some(load) = &loading
            && let Some(result) = load.poll()
        {
            finish_account_switch(app, load.name.clone(), result);
            loading = None;
        }
        if let Some(receiver) = &catalog_load {
            match receiver.try_recv() {
                Ok(done) => {
                    app.finish_catalog(done);
                    catalog_load = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    app.catalog_failed("loader stopped unexpectedly");
                    catalog_load = None;
                }
            }
        }
        // After SIGHUP the terminal is gone; stop drawing and exit cleanly.
        terminal.draw(|frame| ui::draw(frame, app))?;
        if let Some(name) = app.take_account_request() {
            match AccountLoad::start(config, name) {
                Ok(load) => loading = Some(load),
                Err(e) => app.account_load_failed(format!("{e:#}")),
            }
        }
        // The frame above already shows the overlay; fetch behind it.
        if catalog_load.is_none()
            && let Some(job) = app.take_catalog_job()
        {
            match spawn_catalog(job) {
                Ok(receiver) => catalog_load = Some(receiver),
                Err(e) => app.catalog_failed(&format!("{e:#}")),
            }
        }
        let interval = if loading.is_some() || catalog_load.is_some() {
            LOADING_TICK
        } else if app.animating() {
            ANIMATION_TICK
        } else {
            TICK
        };
        if event::poll(interval)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
        app.tick();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use std::cell::RefCell;
    use std::rc::Rc;
    use tuner_app::{Begin, Playback, StreamGate};

    #[derive(Clone, Default)]
    struct FakePlayer(Rc<RefCell<Vec<String>>>);

    impl Playback for FakePlayer {
        fn play(&mut self, _: &str) -> tuner_mpv::Result<()> {
            self.0.borrow_mut().push("play".into());
            Ok(())
        }
        fn shutdown(&mut self) {
            self.0.borrow_mut().push("shutdown".into());
        }
        fn toggle_pause(&mut self) -> tuner_mpv::Result<()> {
            Ok(())
        }
        fn is_running(&mut self) -> bool {
            true
        }
    }

    struct FakeGate;
    impl StreamGate for FakeGate {
        fn begin(&mut self) -> Begin {
            Begin::Go
        }
        fn end(&mut self) {}
    }

    fn config() -> Config {
        let mut config = Config::parse(
            "[[source]]\nname = 'Sample'\ntype = 'm3u'\nlocation = 'unused'\nmax_streams = 3",
        )
        .unwrap();
        config.sources[0].kind = tuner_app::config::SourceKind::M3u {
            location: format!("{}/../../examples/sample.m3u", env!("CARGO_MANIFEST_DIR")),
        };
        config
    }

    #[test]
    fn account_loading_uses_configured_limit_and_m3u_default() {
        let mut config = config();
        let (library, guard, status, _) = load_source(config.resolve(None).unwrap()).unwrap();
        assert!(!library.channels().is_empty());
        assert_eq!(guard.slots(), 3);
        assert!(status.ends_with("limit 3 streams"));
        config.sources[0].max_streams = None;
        let (_, guard, status, _) = load_source(config.resolve(None).unwrap()).unwrap();
        assert_eq!(guard.slots(), 1);
        assert!(status.ends_with("limit 1 stream"));
    }

    fn complete_load(app: &mut App, config: &Config, name: &str) {
        let load = AccountLoad::start(config, name.into()).unwrap();
        let result = load.result.recv_timeout(Duration::from_secs(5)).unwrap();
        finish_account_switch(app, load.name, result);
    }

    #[test]
    fn loading_failure_keeps_current_channels_and_playback() {
        let library = Library::new(
            tuner_core::m3u::parse("#EXTM3U\n#EXTINF:-1,Original\nhttp://original\n").unwrap(),
        );
        let player = FakePlayer::default();
        let mut app = App::new(library, player.clone(), FakeGate);
        app.set_accounts(vec!["Original".into()], Some("Original".into()));
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        let mut config = config();
        config.sources[0].kind = tuner_app::config::SourceKind::M3u {
            location: format!("{}/Cargo.toml/missing.m3u", env!("CARGO_MANIFEST_DIR")),
        };
        app.mode = app::Mode::Loading {
            title: "Switching account",
            name: "Sample".into(),
            started: std::time::Instant::now(),
        };
        complete_load(&mut app, &config, "Sample");
        assert_eq!(app.mode, app::Mode::Normal);
        assert_eq!(app.current_account.as_deref(), Some("Original"));
        assert_eq!(app.now_playing().unwrap().name, "Original");
        assert_eq!(app.visible_len(), 1);
        assert!(app.status.starts_with("Could not switch account:"));
        assert_eq!(*player.0.borrow(), ["play"]);

        // The same event-loop path commits a successful load and stops mpv.
        complete_load(&mut app, &self::config(), "Sample");
        assert_eq!(app.current_account.as_deref(), Some("Sample"));
        assert!(app.now_playing().is_none());
        assert!(app.visible_len() > 1);
        assert_eq!(*player.0.borrow(), ["play", "shutdown"]);
    }

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn picker_renders_current_account_and_scrolls_to_selected_row() {
        let mut app = App::new(Library::default(), FakePlayer::default(), FakeGate);
        app.set_accounts(
            (0..20).map(|i| format!("Account {i:02}")).collect(),
            Some("Account 00".into()),
        );
        app.handle_key(KeyEvent::from(KeyCode::Char('a')));
        let screen = render(&mut app, 80, 12);
        assert!(screen.contains("Accounts"));
        assert!(screen.contains("● Account 00"));
        assert!(screen.contains("enter switch"));
        app.handle_key(KeyEvent::from(KeyCode::End));
        let screen = render(&mut app, 80, 12);
        assert!(screen.contains("Account 19"));
        assert!(!screen.contains("Account 00"));
    }

    #[test]
    fn empty_picker_and_loading_status_render_in_small_terminals() {
        let mut app = App::new(Library::default(), FakePlayer::default(), FakeGate);
        app.handle_key(KeyEvent::from(KeyCode::Char('a')));
        assert!(render(&mut app, 80, 12).contains("No accounts configured."));
        for (width, height) in [(40, 10), (8, 3), (1, 1), (0, 0)] {
            render(&mut app, width, height);
        }
        app.handle_key(KeyEvent::from(KeyCode::Esc));
        app.set_accounts(vec!["Sample".into()], None);
        app.handle_key(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        let screen = render(&mut app, 80, 12);
        assert!(screen.contains("Loading Sample…"));
        assert!(screen.contains("Switching account"));
        assert!(screen.contains("q / Ctrl-C quit"));
        for (width, height) in [(40, 10), (8, 3), (1, 1), (0, 0)] {
            render(&mut app, width, height);
        }
    }

    #[test]
    fn loader_poll_does_not_wait_and_disconnection_clears_loading_state() {
        let (sender, result) = mpsc::channel();
        let load = AccountLoad {
            name: "Sample".into(),
            result,
        };
        assert!(load.poll().is_none());
        drop(sender);
        let mut app = App::new(Library::default(), FakePlayer::default(), FakeGate);
        app.set_accounts(vec!["Sample".into()], None);
        app.handle_key(KeyEvent::from(KeyCode::Char('a')));
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        finish_account_switch(&mut app, load.name.clone(), load.poll().unwrap());
        assert_eq!(app.mode, app::Mode::Normal);
        assert!(app.status.contains("account loader stopped unexpectedly"));
    }

    #[test]
    fn loader_poll_delivers_a_completed_result() {
        let (sender, result) = mpsc::channel();
        let load = AccountLoad {
            name: "Sample".into(),
            result,
        };
        let loaded = load_source(config().resolve(None).unwrap());
        assert!(sender.send(loaded).is_ok());
        let mut app = App::new(Library::default(), FakePlayer::default(), FakeGate);
        finish_account_switch(&mut app, load.name.clone(), load.poll().unwrap());
        assert_eq!(app.current_account.as_deref(), Some("Sample"));
        assert!(!app.rows().is_empty());
    }
}
