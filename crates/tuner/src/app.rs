//! Application state and key handling. Rendering lives in `ui.rs`; this module
//! has no terminal I/O so it can be unit tested with a fake player and gate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use tuner_app::{Catalog, Controller, Playback, Request, StreamGate, Tick, status_line};
use tuner_core::{Channel, Library, Series};
#[cfg(test)]
use tuner_mpv::LoadProgress;
use tuner_mpv::LoadStage;

const PAGE: usize = 10;

/// The [`Controller`] tracks what plays by an index into one list. This app
/// browses several lists, so it keeps the playing [`Channel`] itself and the
/// controller's index is just a marker that something plays.
const SLOT: usize = 0;

/// How long the lists take to wipe in after the view changes.
const REVEAL: Duration = Duration::from_millis(160);

/// How often to ask the provider whether the account is over its limit while
/// something plays.
const WATCH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Channels,
}

/// What the lists show: live TV, or the on-demand movies / series catalogues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Live,
    Movies,
    Series,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Accounts,
    /// A blocking request runs on a worker thread; the loading overlay shows
    /// `title` and "Loading `name`…". Keys are ignored except quit.
    Loading {
        title: &'static str,
        name: String,
        started: Instant,
    },
    /// A popup titled `title` (a stream-limit warning, a stream that could
    /// not be played). With `pending`, the user may confirm (`y`) to play
    /// that channel anyway; without, it is informational.
    Alert {
        title: &'static str,
        message: String,
        pending: Option<Channel>,
    },
}

/// One line of the list pane.
pub struct Row<'a> {
    pub name: &'a str,
    pub playing: bool,
}

/// The highlighted entry, for the details pane.
pub enum Selected<'a> {
    Channel(&'a Channel),
    Series(&'a Series),
}

/// Network work waiting for a worker thread.
enum Load {
    Section(Section),
    Episodes(Series),
}

/// Fetched and indexed off the UI thread, ready to install.
enum Loaded {
    Movies(Library),
    Series(Library<Series>),
    Episodes(Series, Library),
}

/// A catalogue request for a worker thread: [`CatalogJob::run`] blocks.
pub struct CatalogJob {
    load: Load,
    catalog: Arc<dyn Catalog>,
}

/// The outcome of a [`CatalogJob`], handed back to [`App::finish_catalog`].
pub struct CatalogDone {
    load: Load,
    result: anyhow::Result<Loaded>,
}

impl CatalogJob {
    pub fn run(self) -> CatalogDone {
        let catalog = &self.catalog;
        let result = match &self.load {
            Load::Section(Section::Movies) => catalog
                .movies()
                .map(|movies| Loaded::Movies(Library::new(movies))),
            Load::Section(_) => catalog
                .series()
                .map(|series| Loaded::Series(Library::new(series))),
            Load::Episodes(series) => catalog
                .episodes(&series.id)
                .map(|episodes| Loaded::Episodes(series.clone(), Library::new(episodes))),
        };
        CatalogDone {
            load: self.load,
            result,
        }
    }
}

/// A series opened to its episodes, with the series view to return to.
struct Drill {
    series: Series,
    episodes: Library,
    back: Back,
}

struct Back {
    groups: ListState,
    channels: ListState,
    query: String,
}

enum Shown<'a> {
    Channels(&'a Library),
    Series(&'a Library<Series>),
    Empty,
}

pub struct App {
    live: Library,
    /// Movies and series are fetched the first time they are opened.
    movies: Option<Library>,
    series: Option<Library<Series>>,
    drill: Option<Drill>,
    catalog: Option<Arc<dyn Catalog>>,
    pending: Option<Load>,
    section: Section,
    controller: Controller<Box<dyn Playback>, Box<dyn StreamGate>>,
    now: Option<Channel>,
    /// Group list as shown: index 0 is "All", then the shown library's groups.
    group_names: Vec<String>,
    /// Indices into the shown library currently listed.
    visible: Vec<usize>,
    pub groups_state: ListState,
    pub channels_state: ListState,
    pub focus: Focus,
    pub mode: Mode,
    pub query: String,
    pub status: String,
    pub show_details: bool,
    /// Show the full stream URL in the details pane (it embeds the password).
    pub show_url: bool,
    accounts: Vec<String>,
    pub accounts_state: ListState,
    pub current_account: Option<String>,
    account_request: Option<String>,
    /// When the lists last changed, for the wipe-in transition.
    changed_at: Option<Instant>,
    last_watch: Instant,
    /// Load stage last put in the status line, to update it only on change.
    shown_stage: Option<LoadStage>,
    quit: bool,
}

impl App {
    pub fn new(
        library: Library,
        player: impl Playback + 'static,
        gate: impl StreamGate + 'static,
    ) -> Self {
        let status = format!("Loaded {} channels", library.channels().len());
        let mut app = Self {
            live: library,
            movies: None,
            series: None,
            drill: None,
            catalog: None,
            pending: None,
            section: Section::Live,
            controller: Controller::new(Box::new(player), Box::new(gate)),
            now: None,
            group_names: Vec::new(),
            visible: Vec::new(),
            groups_state: ListState::default().with_selected(Some(0)),
            channels_state: ListState::default(),
            focus: Focus::Channels,
            mode: Mode::Normal,
            query: String::new(),
            status,
            show_details: false,
            show_url: false,
            accounts: Vec::new(),
            accounts_state: ListState::default(),
            current_account: None,
            account_request: None,
            changed_at: None,
            last_watch: Instant::now(),
            shown_stage: None,
            quit: false,
        };
        app.reset_view();
        // The first screen needs no transition.
        app.changed_at = None;
        app
    }

    /// Enable the Movies and Series sections, if the source has them.
    pub fn with_catalog(mut self, catalog: impl Catalog + 'static) -> Self {
        self.catalog = Some(Arc::new(catalog));
        self
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// How much of the lists to show, 0.0 to 1.0, while they wipe in after a
    /// section, series or account change. Eased out; 1.0 when idle.
    pub fn reveal(&self) -> f32 {
        self.reveal_at(Instant::now())
    }

    fn reveal_at(&self, now: Instant) -> f32 {
        let Some(changed) = self.changed_at else {
            return 1.0;
        };
        let progress =
            (now.saturating_duration_since(changed).as_secs_f32() / REVEAL.as_secs_f32()).min(1.0);
        1.0 - (1.0 - progress).powi(3)
    }

    /// Whether a transition is running, so the event loop should redraw
    /// quickly.
    pub fn animating(&self) -> bool {
        self.changed_at.is_some_and(|t| t.elapsed() < REVEAL)
    }

    pub fn section(&self) -> Section {
        self.section
    }

    /// Name of the series whose episodes are listed, if one is open.
    pub fn open_series(&self) -> Option<&str> {
        self.drill
            .as_ref()
            .filter(|_| self.section == Section::Series)
            .map(|d| d.series.name.as_str())
    }

    pub fn set_accounts(&mut self, accounts: Vec<String>, current: Option<String>) {
        self.accounts = accounts;
        self.current_account = current;
    }

    pub fn accounts(&self) -> &[String] {
        &self.accounts
    }

    /// The event loop starts a worker after rendering the loading state.
    pub fn take_account_request(&mut self) -> Option<String> {
        self.account_request.take()
    }

    pub fn account_load_failed(&mut self, message: String) {
        self.mode = Mode::Normal;
        self.status = format!("Could not switch account: {message}");
        self.last_watch = Instant::now();
    }

    /// Only commit a switch once the new source has loaded successfully.
    /// Everything of the old account goes: playback, the movie and series
    /// catalogues and any open series.
    pub fn switch_account(
        &mut self,
        name: String,
        library: Library,
        gate: impl StreamGate + 'static,
        catalog: impl Catalog + 'static,
        status: String,
    ) {
        self.controller.replace_gate(Box::new(gate));
        self.now = None;
        self.live = library;
        self.movies = None;
        self.series = None;
        self.drill = None;
        self.pending = None;
        self.catalog = Some(Arc::new(catalog));
        self.section = Section::Live;
        self.reset_view();
        self.mode = Mode::Normal;
        self.current_account = Some(name);
        self.status = status;
        self.last_watch = Instant::now();
    }

    pub fn group_names(&self) -> &[String] {
        &self.group_names
    }

    pub fn rows(&self) -> Vec<Row<'_>> {
        match self.shown() {
            Shown::Channels(library) => self
                .visible
                .iter()
                .map(|&i| {
                    let c = &library.channels()[i];
                    Row {
                        name: &c.name,
                        playing: self.is_playing(c),
                    }
                })
                .collect(),
            Shown::Series(library) => self
                .visible
                .iter()
                .map(|&i| Row {
                    name: &library.items()[i].name,
                    playing: false,
                })
                .collect(),
            Shown::Empty => Vec::new(),
        }
    }

    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub fn now_playing(&self) -> Option<&Channel> {
        self.now.as_ref()
    }

    pub fn selected(&self) -> Option<Selected<'_>> {
        let i = *self.visible.get(self.channels_state.selected()?)?;
        match self.shown() {
            Shown::Channels(library) => library.channels().get(i).map(Selected::Channel),
            Shown::Series(library) => library.items().get(i).map(Selected::Series),
            Shown::Empty => None,
        }
    }

    pub fn is_playing(&self, channel: &Channel) -> bool {
        self.now.as_ref().is_some_and(|c| c.url == channel.url)
    }

    /// Periodic housekeeping: notice when the user closed the mpv window or
    /// the stream failed, so the stream slot is released for other
    /// windows/devices.
    pub fn tick(&mut self) {
        if let Some(tick) = self.controller.tick() {
            let name = self.now.take().map(|c| c.name).unwrap_or_default();
            match tick {
                Tick::Closed => self.status = "Player closed".into(),
                Tick::Ended => self.status = format!("{name} ended"),
                Tick::Failed(reason) => {
                    self.status = format!("Could not play {name}");
                    // Don't cover a popup or the loader; the status says it.
                    if self.mode == Mode::Normal {
                        self.mode = Mode::Alert {
                            title: "Stream unavailable",
                            message: format!("{name}: {reason}. The player was closed."),
                            pending: None,
                        };
                    }
                }
            }
        } else if let Some(channel) = &self.now {
            // Step, progress and elapsed time until the first frame (mpv has
            // no window before), then "Playing" once.
            let progress = self.controller.progress();
            let stage = progress.as_ref().map(|p| p.stage);
            let opening = stage.is_some_and(|s| s != LoadStage::Playing);
            if opening || stage != self.shown_stage {
                self.shown_stage = stage;
                self.status = status_line(&channel.name, progress.as_ref());
            }
        }
        // Provider checks are blocking; defer them while the loader animates.
        if !matches!(self.mode, Mode::Loading { .. }) && self.last_watch.elapsed() >= WATCH_INTERVAL
        {
            self.last_watch = Instant::now();
            self.watch();
        }
    }

    /// The catalogue request started by the last key, if any. The event loop
    /// runs it on a worker thread, after drawing the loading overlay, and
    /// hands the outcome to [`App::finish_catalog`].
    pub fn take_catalog_job(&mut self) -> Option<CatalogJob> {
        let load = self.pending.take()?;
        let catalog = self.catalog.clone()?;
        Some(CatalogJob { load, catalog })
    }

    pub fn finish_catalog(&mut self, done: CatalogDone) {
        self.mode = Mode::Normal;
        self.last_watch = Instant::now();
        match done.result {
            Ok(Loaded::Movies(movies)) => {
                self.status = format!("Loaded {} movies", movies.items().len());
                self.movies = Some(movies);
                self.enter_section(Section::Movies);
            }
            Ok(Loaded::Series(series)) => {
                self.status = format!("Loaded {} series", series.items().len());
                self.series = Some(series);
                self.enter_section(Section::Series);
            }
            Ok(Loaded::Episodes(series, episodes)) if episodes.items().is_empty() => {
                self.status = format!("No episodes found for {}", series.name);
            }
            Ok(Loaded::Episodes(series, episodes)) => {
                self.status = format!("{}: {} episodes", series.name, episodes.items().len());
                let back = Back {
                    groups: self.groups_state.clone(),
                    channels: self.channels_state.clone(),
                    query: std::mem::take(&mut self.query),
                };
                self.drill = Some(Drill {
                    series,
                    episodes,
                    back,
                });
                self.reset_view();
            }
            Err(e) => {
                self.status = format!(
                    "Could not load {}: {e:#}",
                    match done.load {
                        Load::Section(Section::Movies) => "movies",
                        Load::Section(_) => "series",
                        Load::Episodes(_) => "episodes",
                    }
                );
            }
        }
    }

    /// The worker vanished without an answer.
    pub fn catalog_failed(&mut self, message: &str) {
        self.mode = Mode::Normal;
        self.status = format!("Could not load catalogue: {message}");
    }

    /// Start the request on the next loop turn and show the loading overlay.
    fn begin_load(&mut self, load: Load) {
        let (title, name) = match &load {
            Load::Section(Section::Movies) => ("Movies", "movies".to_string()),
            Load::Section(_) => ("Series", "series".to_string()),
            Load::Episodes(series) => ("Episodes", series.name.clone()),
        };
        self.status = format!("Loading {name}…");
        self.mode = Mode::Loading {
            title,
            name,
            started: Instant::now(),
        };
        self.pending = Some(load);
    }

    #[cfg(test)]
    fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Run the waiting request on this thread, as the event loop would on a
    /// worker.
    #[cfg(test)]
    fn run_pending(&mut self) {
        if let Some(job) = self.take_catalog_job() {
            let done = job.run();
            self.finish_catalog(done);
        }
    }

    /// Ask the provider whether another device pushed the account over its
    /// limit while we play, and warn if so.
    fn watch(&mut self) {
        if let Some(message) = self.controller.watch() {
            self.status = "⚠ Account over its stream limit".into();
            if self.mode == Mode::Normal {
                self.mode = Mode::Alert {
                    title: "Stream limit",
                    message,
                    pending: None,
                };
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.mode {
            Mode::Search => self.handle_search_key(key),
            Mode::Normal => self.handle_normal_key(key),
            Mode::Accounts => self.handle_accounts_key(key),
            Mode::Loading { .. } => {
                if key.code == KeyCode::Char('q') {
                    self.quit = true;
                }
            }
            Mode::Alert { .. } => self.handle_alert_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('1') => self.switch_section(Section::Live),
            KeyCode::Char('2') => self.switch_section(Section::Movies),
            KeyCode::Char('3') => self.switch_section(Section::Series),
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(PAGE as isize),
            KeyCode::PageUp => self.move_by(-(PAGE as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.move_by(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_by(isize::MAX),
            KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
            KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::Groups,
            KeyCode::Char('l') | KeyCode::Right => self.focus = Focus::Channels,
            KeyCode::Char('i') => self.show_details = !self.show_details,
            KeyCode::Char('u') => self.show_url = !self.show_url,
            KeyCode::Char('a') => {
                let selected = self
                    .current_account
                    .as_ref()
                    .and_then(|name| self.accounts.iter().position(|n| n == name))
                    .or_else(|| (!self.accounts.is_empty()).then_some(0));
                self.accounts_state = ListState::default().with_selected(selected);
                self.mode = Mode::Accounts;
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.focus = Focus::Channels;
            }
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Esc | KeyCode::Backspace if self.open_series().is_some() => {
                self.close_series();
            }
            KeyCode::Enter => match self.focus {
                Focus::Groups => self.focus = Focus::Channels,
                Focus::Channels => self.open_selected(),
            },
            KeyCode::Char(' ') => {
                self.status = match self.controller.toggle_pause() {
                    Ok(()) => "Toggled pause".into(),
                    Err(e) => format!("Error: {e}"),
                }
            }
            KeyCode::Char('s') => self.stop(),
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.query.clear();
                self.refilter();
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                self.open_selected();
            }
            KeyCode::Down => self.move_by(1),
            KeyCode::Up => self.move_by(-1),
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.refilter();
            }
            _ => {}
        }
    }

    fn handle_accounts_key(&mut self, key: KeyEvent) {
        let delta = match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'a') => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                if let Some(name) = self
                    .accounts_state
                    .selected()
                    .and_then(|i| self.accounts.get(i))
                {
                    if self.current_account.as_ref() != Some(name) {
                        self.account_request = Some(name.clone());
                        self.status = format!("Loading {name}…");
                        self.mode = Mode::Loading {
                            title: "Switching account",
                            name: name.clone(),
                            started: Instant::now(),
                        };
                    } else {
                        self.mode = Mode::Normal;
                    }
                }
                return;
            }
            KeyCode::Char('j') | KeyCode::Down => 1,
            KeyCode::Char('k') | KeyCode::Up => -1,
            KeyCode::PageDown => PAGE as isize,
            KeyCode::PageUp => -(PAGE as isize),
            KeyCode::Char('g') | KeyCode::Home => isize::MIN,
            KeyCode::Char('G') | KeyCode::End => isize::MAX,
            _ => return,
        };
        if !self.accounts.is_empty() {
            let next = self
                .accounts_state
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(self.accounts.len() - 1);
            self.accounts_state.select(Some(next));
        }
    }

    fn handle_alert_key(&mut self, key: KeyEvent) {
        let Mode::Alert { pending, .. } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        match (pending, key.code) {
            (Some(channel), KeyCode::Char('y' | 'Y')) => {
                let outcome = self.controller.confirm(SLOT, &channel.url);
                self.show_outcome(channel, outcome);
            }
            (Some(_), _) => {
                self.controller.cancel();
                self.status = "Cancelled: nothing was opened".into();
            }
            (None, _) => {}
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Groups => Focus::Channels,
            Focus::Channels => Focus::Groups,
        };
    }

    /// Move the selection in the focused list. Saturates at both ends.
    fn move_by(&mut self, delta: isize) {
        let (state, len) = match self.focus {
            Focus::Groups => (&mut self.groups_state, self.group_names.len()),
            Focus::Channels => (&mut self.channels_state, self.visible.len()),
        };
        if len == 0 {
            return;
        }
        let current = state.selected().unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(len - 1);
        state.select(Some(next));
        if self.focus == Focus::Groups {
            self.refilter();
        }
    }

    fn shown(&self) -> Shown<'_> {
        match self.section {
            Section::Live => Shown::Channels(&self.live),
            Section::Movies => self.movies.as_ref().map_or(Shown::Empty, Shown::Channels),
            Section::Series => match &self.drill {
                Some(drill) => Shown::Channels(&drill.episodes),
                None => self.series.as_ref().map_or(Shown::Empty, Shown::Series),
            },
        }
    }

    fn selected_group(&self) -> Option<&str> {
        match self.groups_state.selected() {
            None | Some(0) => None,
            Some(i) => self.group_names.get(i).map(String::as_str),
        }
    }

    fn matching(&self) -> Vec<usize> {
        let group = self.selected_group();
        match self.shown() {
            Shown::Channels(library) => library.filter(&self.query, group),
            Shown::Series(library) => library.filter(&self.query, group),
            Shown::Empty => Vec::new(),
        }
    }

    fn refilter(&mut self) {
        self.visible = self.matching();
        let selected = (!self.visible.is_empty()).then_some(0);
        self.channels_state = ListState::default().with_selected(selected);
    }

    fn rebuild_groups(&mut self) {
        let groups = match self.shown() {
            Shown::Channels(library) => library.groups(),
            Shown::Series(library) => library.groups(),
            Shown::Empty => &[],
        };
        self.group_names = std::iter::once("All".to_string())
            .chain(groups.iter().cloned())
            .collect();
    }

    /// The shown library changed: start at its first group and entry.
    fn reset_view(&mut self) {
        self.rebuild_groups();
        self.groups_state = ListState::default().with_selected(Some(0));
        self.query.clear();
        self.focus = Focus::Channels;
        self.refilter();
        self.changed_at = Some(Instant::now());
    }

    fn switch_section(&mut self, section: Section) {
        if section == self.section {
            return;
        }
        if section != Section::Live && !self.catalog.as_ref().is_some_and(|c| c.has_vod()) {
            self.status = "Movies and series need an Xtream source".into();
            return;
        }
        let loaded = match section {
            Section::Live => true,
            Section::Movies => self.movies.is_some(),
            Section::Series => self.series.is_some(),
        };
        if loaded {
            self.enter_section(section);
        } else {
            self.begin_load(Load::Section(section));
        }
    }

    fn enter_section(&mut self, section: Section) {
        self.section = section;
        self.reset_view();
    }

    /// Leave a series' episodes for the series list, as it was left.
    fn close_series(&mut self) {
        let Some(drill) = self.drill.take() else {
            return;
        };
        self.rebuild_groups();
        self.groups_state = drill.back.groups;
        self.query = drill.back.query;
        self.visible = self.matching();
        self.channels_state = drill.back.channels;
        self.status = format!("Back to series ({})", self.visible.len());
        self.changed_at = Some(Instant::now());
    }

    /// Enter on the selection: play a channel, movie or episode; open a
    /// series to its episodes.
    fn open_selected(&mut self) {
        match self.selected() {
            Some(Selected::Channel(channel)) => {
                let channel = channel.clone();
                self.play(channel);
            }
            Some(Selected::Series(series)) => {
                let series = series.clone();
                self.begin_load(Load::Episodes(series));
            }
            None => {}
        }
    }

    /// Play a channel. The shared [`Controller`] decides whether the stream
    /// limit needs checking (only when nothing is playing).
    fn play(&mut self, channel: Channel) {
        let outcome = self.controller.request(SLOT, &channel.url);
        self.show_outcome(channel, outcome);
    }

    fn show_outcome(&mut self, channel: Channel, outcome: Request) {
        match outcome {
            Request::Started => {
                let progress = self.controller.progress();
                self.shown_stage = progress.as_ref().map(|p| p.stage);
                self.status = status_line(&channel.name, progress.as_ref());
                self.now = Some(channel);
            }
            Request::NeedsConfirm(message) => {
                self.mode = Mode::Alert {
                    title: "Stream limit: confirm",
                    message,
                    pending: Some(channel),
                }
            }
            Request::Blocked(message) => {
                self.status = "Blocked: stream limit reached".into();
                self.mode = Mode::Alert {
                    title: "Stream limit",
                    message,
                    pending: None,
                };
            }
            Request::Failed(e) => self.status = format!("Error: {e}"),
        }
    }

    fn stop(&mut self) {
        self.controller.stop();
        self.now = None;
        self.status = "Stopped".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use tuner_app::Begin;

    type Log = Rc<RefCell<Vec<String>>>;

    #[derive(Clone, Default)]
    struct FakePlayer {
        log: Log,
        closed: Rc<RefCell<bool>>,
        end: Rc<RefCell<Option<tuner_mpv::StreamEnd>>>,
        progress: Rc<RefCell<Option<LoadProgress>>>,
    }

    impl Playback for FakePlayer {
        fn play(&mut self, url: &str) -> tuner_mpv::Result<()> {
            self.log.borrow_mut().push(format!("play {url}"));
            *self.closed.borrow_mut() = false;
            Ok(())
        }
        fn shutdown(&mut self) {
            self.log.borrow_mut().push("shutdown".into());
        }
        fn toggle_pause(&mut self) -> tuner_mpv::Result<()> {
            self.log.borrow_mut().push("pause".into());
            Ok(())
        }
        fn is_running(&mut self) -> bool {
            !*self.closed.borrow()
        }
        fn poll_end(&mut self) -> Option<tuner_mpv::StreamEnd> {
            self.end.borrow_mut().take()
        }
        fn progress(&self) -> Option<LoadProgress> {
            self.progress.borrow().clone()
        }
    }

    /// Answers `begin` with a fixed verdict and records calls in the shared log.
    struct FakeGate {
        verdict: Begin,
        log: Log,
        warning: Option<String>,
    }

    impl StreamGate for FakeGate {
        fn begin(&mut self) -> Begin {
            self.log.borrow_mut().push("begin".into());
            self.verdict.clone()
        }
        fn end(&mut self) {
            self.log.borrow_mut().push("end".into());
        }
        fn watch(&mut self) -> Option<String> {
            self.warning.clone()
        }
    }

    fn app_with(verdict: Begin) -> (App, FakePlayer) {
        app_watching(verdict, None)
    }

    fn app_watching(verdict: Begin, warning: Option<String>) -> (App, FakePlayer) {
        let input = "#EXTM3U\n\
            #EXTINF:-1 group-title=\"News\",BBC News\nhttp://bbc\n\
            #EXTINF:-1 group-title=\"Sports\",Sky Sports\nhttp://sky\n\
            #EXTINF:-1 group-title=\"News\",CNN\nhttp://cnn\n";
        let player = FakePlayer::default();
        let gate = FakeGate {
            verdict,
            log: player.log.clone(),
            warning,
        };
        let lib = Library::new(tuner_core::m3u::parse(input).unwrap());
        (App::new(lib, player.clone(), gate), player)
    }

    fn app() -> (App, FakePlayer) {
        app_with(Begin::Go)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::from(code));
    }

    fn names(app: &App) -> Vec<&str> {
        app.rows().into_iter().map(|r| r.name).collect()
    }

    fn log(player: &FakePlayer) -> Vec<String> {
        player.log.borrow().clone()
    }

    #[test]
    fn navigates_and_plays() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(log(&player), ["begin", "play http://sky"]);
        assert_eq!(app.now_playing().unwrap().name, "Sky Sports");
    }

    #[test]
    fn switching_channels_does_not_open_a_second_slot() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            log(&player),
            ["begin", "play http://bbc", "play http://sky"]
        );
    }

    #[test]
    fn stop_releases_the_slot() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(
            log(&player),
            ["begin", "play http://bbc", "shutdown", "end"]
        );
        assert!(app.now_playing().is_none());
    }

    #[test]
    fn closing_mpv_releases_the_slot() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Enter);
        *player.closed.borrow_mut() = true;
        app.tick();
        assert!(app.now_playing().is_none());
        assert_eq!(log(&player).last().map(String::as_str), Some("end"));
        app.tick();
        assert_eq!(log(&player).iter().filter(|l| *l == "end").count(), 1);
    }

    #[test]
    fn unreachable_stream_closes_mpv_and_explains() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Enter);
        *player.end.borrow_mut() = Some(tuner_mpv::StreamEnd::Failed("loading failed".into()));
        app.tick();
        assert!(app.now_playing().is_none());
        assert_eq!(
            log(&player),
            ["begin", "play http://bbc", "shutdown", "end"]
        );
        assert_eq!(app.status, "Could not play BBC News");
        assert!(matches!(
            &app.mode,
            Mode::Alert { title: "Stream unavailable", pending: None, message }
                if message.contains("loading failed")
        ));
        // Any key dismisses it; playing again checks the limit again.
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        press(&mut app, KeyCode::Enter);
        assert_eq!(log(&player).last().unwrap(), "play http://bbc");
    }

    #[test]
    fn status_shows_progress_until_playing() {
        let (mut app, player) = app();
        let at = |stage, secs| {
            Some(LoadProgress {
                stage,
                elapsed: Duration::from_secs(secs),
                bytes: None,
                buffered: None,
                problem: None,
            })
        };
        *player.progress.borrow_mut() = at(LoadStage::Connecting, 0);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.status, "Connecting to BBC News… · step 1/2 · 0 s");
        // Refreshed every tick while opening, so the seconds count up.
        *player.progress.borrow_mut() = at(LoadStage::Connecting, 3);
        app.tick();
        assert_eq!(app.status, "Connecting to BBC News… · step 1/2 · 3 s");
        *player.progress.borrow_mut() = at(LoadStage::Buffering, 5);
        app.tick();
        assert_eq!(app.status, "Buffering BBC News… · step 2/2 · 5 s");
        *player.progress.borrow_mut() = at(LoadStage::Playing, 6);
        app.tick();
        assert_eq!(app.status, "Playing BBC News");
        // Once playing, other messages stay.
        app.status = "something else".into();
        *player.progress.borrow_mut() = at(LoadStage::Playing, 9);
        app.tick();
        assert_eq!(app.status, "something else");
    }

    #[test]
    fn ended_stream_only_updates_the_status() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Enter);
        *player.end.borrow_mut() = Some(tuner_mpv::StreamEnd::Ended);
        app.tick();
        assert!(app.now_playing().is_none());
        assert_eq!(app.status, "BBC News ended");
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn blocked_never_plays() {
        let (mut app, player) = app_with(Begin::Blocked("busy".into()));
        press(&mut app, KeyCode::Enter);
        assert!(
            matches!(&app.mode, Mode::Alert { pending: None, message, .. } if message == "busy")
        );
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(log(&player), ["begin"]);
    }

    #[test]
    fn confirm_plays_only_on_y() {
        let (mut app, player) = app_with(Begin::Confirm("full".into()));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(log(&player), ["begin", "end"]);
        assert!(app.now_playing().is_none());

        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(log(&player)[2..], ["begin", "play http://bbc"]);
    }

    #[test]
    fn watch_warns_while_playing_only() {
        let (mut app, _) = app_watching(Begin::Go, Some("over".into()));
        app.watch();
        assert_eq!(app.mode, Mode::Normal, "nothing playing yet");
        press(&mut app, KeyCode::Enter);
        app.watch();
        assert!(
            matches!(&app.mode, Mode::Alert { pending: None, message, .. } if message == "over")
        );
    }

    #[test]
    fn i_toggles_details_for_selection() {
        let (mut app, _) = app();
        press(&mut app, KeyCode::Char('i'));
        assert!(app.show_details);
        press(&mut app, KeyCode::Char('j'));
        assert!(matches!(
            app.selected(),
            Some(Selected::Channel(c)) if c.name == "Sky Sports"
        ));
    }

    #[test]
    fn selection_saturates() {
        let (mut app, _) = app();
        press(&mut app, KeyCode::Char('G'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.channels_state.selected(), Some(2));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.channels_state.selected(), Some(0));
    }

    #[test]
    fn group_selection_filters_channels() {
        let (mut app, _) = app();
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(names(&app), ["BBC News", "CNN"]);
    }

    #[test]
    fn search_filters_and_esc_clears() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Char('/'));
        for c in "cnn".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(names(&app), ["CNN"]);
        press(&mut app, KeyCode::Enter);
        assert_eq!(log(&player), ["begin", "play http://cnn"]);
        assert_eq!(app.mode, Mode::Normal);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visible_len(), 3);
    }

    #[test]
    fn q_quits_in_normal_mode_but_types_in_search() {
        let (mut app, _) = app();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit());
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit());
    }

    type Calls = Arc<std::sync::Mutex<Vec<String>>>;

    /// Serves a tiny catalogue and records which requests were made.
    struct FakeCatalog {
        vod: bool,
        fail: bool,
        calls: Calls,
    }

    impl Catalog for FakeCatalog {
        fn has_vod(&self) -> bool {
            self.vod
        }
        fn movies_batched(
            &self,
            on_batch: &mut dyn FnMut(Vec<Channel>) -> tuner_app::Flow,
        ) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push("movies".into());
            if self.fail {
                anyhow::bail!("server returned HTTP 500");
            }
            let mut heat = Channel::new("Heat", "http://heat.mkv");
            heat.group = Some("Action".into());
            let mut up = Channel::new("Up", "http://up.mp4");
            up.group = Some("Family".into());
            // Two parts, as a real download would arrive.
            let _ = on_batch(vec![heat]);
            let _ = on_batch(vec![up]);
            Ok(())
        }
        fn series_batched(
            &self,
            on_batch: &mut dyn FnMut(Vec<Series>) -> tuner_app::Flow,
        ) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push("series".into());
            let show = |id: &str, name: &str| Series {
                id: id.into(),
                name: name.into(),
                group: Some("Drama".into()),
                cover: None,
                plot: Some("A show.".into()),
                year: None,
                rating: None,
            };
            let _ = on_batch(vec![show("1", "The Wire"), show("2", "Lost")]);
            Ok(())
        }
        fn episodes(&self, series_id: &str) -> anyhow::Result<Vec<Channel>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("episodes {series_id}"));
            let episode = |name: &str, season: u32| {
                let mut e = Channel::new(name, format!("http://{name}"));
                e.group = Some(format!("Season {season}"));
                e
            };
            Ok(vec![
                episode("s1e1", 1),
                episode("s1e2", 1),
                episode("s2e1", 2),
            ])
        }
    }

    fn vod_app(vod: bool, fail: bool) -> (App, FakePlayer, Calls) {
        let (app, player) = app();
        let calls = Calls::default();
        let catalog = FakeCatalog {
            vod,
            fail,
            calls: calls.clone(),
        };
        (app.with_catalog(catalog), player, calls)
    }

    /// Press a key and finish any catalogue request it started.
    fn press_and_load(app: &mut App, code: KeyCode) {
        press(app, code);
        app.run_pending();
    }

    #[test]
    fn vod_sections_need_a_catalog_with_vod() {
        let (mut app, _) = app();
        press(&mut app, KeyCode::Char('2'));
        assert!(!app.has_pending());
        assert_eq!(app.section(), Section::Live);
        assert!(app.status.contains("Xtream"));

        let (mut app, _, calls) = vod_app(false, false);
        press(&mut app, KeyCode::Char('3'));
        assert!(!app.has_pending());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn movies_load_on_first_visit_only() {
        let (mut app, _, calls) = vod_app(true, false);
        press(&mut app, KeyCode::Char('2'));
        assert!(app.has_pending(), "request waits for the next draw");
        assert_eq!(app.section(), Section::Live);
        assert_eq!(app.status, "Loading movies…");
        app.run_pending();
        assert_eq!(app.section(), Section::Movies);
        assert_eq!(names(&app), ["Heat", "Up"]);
        assert_eq!(app.group_names(), ["All", "Action", "Family"]);

        press(&mut app, KeyCode::Char('1'));
        assert_eq!(names(&app), ["BBC News", "Sky Sports", "CNN"]);
        press_and_load(&mut app, KeyCode::Char('2'));
        assert_eq!(names(&app), ["Heat", "Up"]);
        assert_eq!(*calls.lock().unwrap(), ["movies"]);
    }

    #[test]
    fn failed_load_stays_put_and_can_be_retried() {
        let (mut app, _, calls) = vod_app(true, true);
        press_and_load(&mut app, KeyCode::Char('2'));
        assert_eq!(app.section(), Section::Live);
        assert_eq!(
            app.status,
            "Could not load movies: server returned HTTP 500"
        );
        press_and_load(&mut app, KeyCode::Char('2'));
        assert_eq!(*calls.lock().unwrap(), ["movies", "movies"]);
    }

    #[test]
    fn plays_a_movie_and_keeps_playing_across_sections() {
        let (mut app, player, _) = vod_app(true, false);
        press_and_load(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(log(&player), ["begin", "play http://heat.mkv"]);
        assert_eq!(app.now_playing().unwrap().name, "Heat");
        assert!(app.rows()[0].playing);

        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.now_playing().unwrap().name, "Heat");
        assert!(app.rows().iter().all(|r| !r.playing));
        press(&mut app, KeyCode::Enter);
        assert_eq!(log(&player).last().unwrap(), "play http://bbc");
        assert_eq!(
            log(&player).iter().filter(|l| *l == "begin").count(),
            1,
            "switching reuses the stream slot"
        );
    }

    #[test]
    fn confirming_a_movie_plays_it() {
        let (app, player) = app_with(Begin::Confirm("full".into()));
        let mut app = app.with_catalog(FakeCatalog {
            vod: true,
            fail: false,
            calls: Calls::default(),
        });
        press_and_load(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(&app.mode, Mode::Alert { pending: Some(c), .. } if c.name == "Heat"));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(log(&player).last().unwrap(), "play http://heat.mkv");
        assert_eq!(app.now_playing().unwrap().name, "Heat");
    }

    #[test]
    fn series_open_to_episodes_grouped_by_season_and_back() {
        let (mut app, player, calls) = vod_app(true, false);
        press_and_load(&mut app, KeyCode::Char('3'));
        assert_eq!(names(&app), ["The Wire", "Lost"]);
        press(&mut app, KeyCode::Char('j'));
        assert!(matches!(
            app.selected(),
            Some(Selected::Series(s)) if s.name == "Lost"
        ));

        press_and_load(&mut app, KeyCode::Enter);
        assert_eq!(*calls.lock().unwrap(), ["series", "episodes 2"]);
        assert_eq!(app.open_series(), Some("Lost"));
        assert_eq!(names(&app), ["s1e1", "s1e2", "s2e1"]);
        assert_eq!(app.group_names(), ["All", "Season 1", "Season 2"]);

        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(names(&app), ["s2e1"]);
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(log(&player), ["begin", "play http://s2e1"]);

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.open_series(), None);
        assert_eq!(names(&app), ["The Wire", "Lost"]);
        assert_eq!(app.channels_state.selected(), Some(1), "selection restored");
        assert_eq!(app.now_playing().unwrap().name, "s2e1");
    }

    #[test]
    fn esc_clears_a_search_before_leaving_a_series() {
        let (mut app, _, _) = vod_app(true, false);
        press_and_load(&mut app, KeyCode::Char('3'));
        press_and_load(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('/'));
        for c in "s2".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            app.open_series(),
            Some("The Wire"),
            "Esc in search only clears it"
        );
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.open_series(), None);
    }

    #[test]
    fn episodes_stay_open_when_leaving_and_returning_to_series() {
        let (mut app, _, calls) = vod_app(true, false);
        press_and_load(&mut app, KeyCode::Char('3'));
        press_and_load(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.open_series(), None, "only shown in the Series section");
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.open_series(), Some("The Wire"));
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    fn configure_accounts(app: &mut App) {
        app.set_accounts(
            vec!["Home".into(), "Travel".into(), "Free".into()],
            Some("Travel".into()),
        );
    }

    #[test]
    fn account_picker_selects_current_and_cancels_without_affecting_playback() {
        let (mut app, player) = app();
        configure_accounts(&mut app);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.mode, Mode::Accounts);
        assert_eq!(app.accounts_state.selected(), Some(1));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.take_account_request().is_none());
        assert_eq!(app.current_account.as_deref(), Some("Travel"));
        assert_eq!(log(&player), ["begin", "play http://bbc"]);
    }

    #[test]
    fn account_picker_requests_selected_account_without_playing_it() {
        let (mut app, player) = app();
        configure_accounts(&mut app);
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('G'));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.accounts_state.selected(), Some(2));
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.accounts_state.selected(), Some(0));
        press(&mut app, KeyCode::PageDown);
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.take_account_request().as_deref(), Some("Home"));
        assert!(app.take_account_request().is_none());
        assert!(matches!(&app.mode, Mode::Loading { name, .. } if name == "Home"));
        assert_eq!(app.current_account.as_deref(), Some("Travel"));
        assert!(log(&player).is_empty());
    }

    #[test]
    fn loading_prevents_duplicate_requests_and_keeps_quit_responsive() {
        let (mut app, player) = app();
        configure_accounts(&mut app);
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.take_account_request().as_deref(), Some("Home"));
        for code in [KeyCode::Enter, KeyCode::Char('a'), KeyCode::Esc] {
            press(&mut app, code);
        }
        assert!(matches!(app.mode, Mode::Loading { .. }));
        assert!(app.take_account_request().is_none());
        assert!(log(&player).is_empty());
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit());
    }

    #[test]
    fn loading_defers_blocking_provider_watch_but_still_notices_player_close() {
        let (mut app, player) = app_watching(Begin::Go, Some("over".into()));
        configure_accounts(&mut app);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::Enter);
        app.last_watch = Instant::now() - WATCH_INTERVAL;
        app.tick();
        assert!(app.status.starts_with("Loading"));
        assert!(matches!(app.mode, Mode::Loading { .. }));
        *player.closed.borrow_mut() = true;
        app.tick();
        assert!(app.now_playing().is_none());
        assert_eq!(log(&player).last().unwrap(), "end");
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit());
    }

    #[test]
    fn selecting_current_account_is_a_noop() {
        let (mut app, player) = app();
        configure_accounts(&mut app);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.take_account_request().is_none());
        assert_eq!(log(&player), ["begin", "play http://bbc"]);
    }

    #[test]
    fn empty_account_picker_is_safe_and_ctrl_c_still_quits() {
        let (mut app, player) = app();
        press(&mut app, KeyCode::Char('a'));
        for key in [KeyCode::Down, KeyCode::End, KeyCode::Enter] {
            press(&mut app, key);
        }
        assert_eq!(app.mode, Mode::Accounts);
        assert_eq!(app.accounts_state.selected(), None);
        assert!(app.take_account_request().is_none());
        assert!(log(&player).is_empty());
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit());
    }

    #[test]
    fn account_switch_resets_browsing_and_enforces_new_gate() {
        let (mut app, player) = app();
        configure_accounts(&mut app);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('b'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('h'));
        let library = Library::new(
            tuner_core::m3u::parse("#EXTM3U\n#EXTINF:-1 group-title=\"New\",Other\nhttp://other\n")
                .unwrap(),
        );
        app.switch_account(
            "Home".into(),
            library,
            FakeGate {
                verdict: Begin::Blocked("new account full".into()),
                log: player.log.clone(),
                warning: None,
            },
            FakeCatalog {
                vod: false,
                fail: false,
                calls: Calls::default(),
            },
            "New account loaded".into(),
        );
        assert!(app.now_playing().is_none());
        assert!(app.query.is_empty());
        assert_eq!(app.group_names(), ["All", "New"]);
        assert_eq!(app.groups_state.selected(), Some(0));
        assert_eq!(app.channels_state.selected(), Some(0));
        assert_eq!(app.focus, Focus::Channels);
        assert_eq!(names(&app), ["Other"]);
        assert_eq!(app.current_account.as_deref(), Some("Home"));
        assert_eq!(log(&player)[2..], ["play http://bbc", "shutdown", "end"]);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Alert { pending: None, .. }));
        assert_eq!(log(&player).last().unwrap(), "begin");
        assert!(app.now_playing().is_none());
    }

    #[test]
    fn account_switch_drops_the_old_accounts_movies_and_series() {
        let (mut app, _, _) = vod_app(true, false);
        configure_accounts(&mut app);
        press_and_load(&mut app, KeyCode::Char('3'));
        press_and_load(&mut app, KeyCode::Enter);
        assert_eq!(app.open_series(), Some("The Wire"));

        let new_calls = Calls::default();
        let library = Library::new(
            tuner_core::m3u::parse("#EXTM3U\n#EXTINF:-1,Other\nhttp://other\n").unwrap(),
        );
        app.switch_account(
            "Home".into(),
            library,
            FakeGate {
                verdict: Begin::Go,
                log: Log::default(),
                warning: None,
            },
            FakeCatalog {
                vod: true,
                fail: false,
                calls: new_calls.clone(),
            },
            "switched".into(),
        );
        assert_eq!(app.section(), Section::Live);
        assert_eq!(app.open_series(), None);
        assert_eq!(names(&app), ["Other"]);

        // The new account's catalogue is fetched afresh.
        press_and_load(&mut app, KeyCode::Char('2'));
        assert_eq!(*new_calls.lock().unwrap(), ["movies"]);
        assert_eq!(names(&app), ["Heat", "Up"]);
    }

    #[test]
    fn catalogue_loading_shows_the_overlay_and_ignores_keys_until_done() {
        let (mut app, _, _) = vod_app(true, false);
        press(&mut app, KeyCode::Char('2'));
        assert!(matches!(
            &app.mode,
            Mode::Loading { title: "Movies", name, .. } if name == "movies"
        ));
        for code in [KeyCode::Char('3'), KeyCode::Char('j'), KeyCode::Enter] {
            press(&mut app, code);
        }
        assert_eq!(app.section(), Section::Live);

        let job = app.take_catalog_job().expect("the key queued a request");
        assert!(app.take_catalog_job().is_none(), "taken once");
        assert!(matches!(app.mode, Mode::Loading { .. }), "until it is done");
        app.finish_catalog(job.run());
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.section(), Section::Movies);
    }

    #[test]
    fn opening_a_series_shows_its_name_while_loading() {
        let (mut app, _, _) = vod_app(true, false);
        press_and_load(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.mode,
            Mode::Loading { title: "Episodes", name, .. } if name == "The Wire"
        ));
        app.run_pending();
        assert_eq!(app.open_series(), Some("The Wire"));
    }

    #[test]
    fn quit_works_while_a_catalogue_loads_and_a_lost_worker_recovers() {
        let (mut app, _, _) = vod_app(true, false);
        press(&mut app, KeyCode::Char('2'));
        app.catalog_failed("worker stopped");
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status.contains("worker stopped"));

        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit());
    }

    #[test]
    fn views_wipe_in_after_a_change_but_not_at_start() {
        let (mut app, _, _) = vod_app(true, false);
        assert_eq!(app.reveal(), 1.0);
        assert!(!app.animating());

        press_and_load(&mut app, KeyCode::Char('2'));
        assert!(app.animating());
        let t0 = app.changed_at.expect("a change starts the transition");
        let steps: Vec<f32> = [0, 40, 80, 120, 160, 400]
            .map(|ms| app.reveal_at(t0 + Duration::from_millis(ms)))
            .into();
        assert_eq!(steps[0], 0.0);
        assert!(
            steps.windows(2).all(|w| w[0] <= w[1]),
            "never goes backwards"
        );
        assert!(steps[1] > 0.0 && steps[1] < 1.0);
        assert_eq!(steps[4], 1.0);
        assert_eq!(steps[5], 1.0);
        // Before the change time (clock skew) is treated as the start.
        assert_eq!(app.reveal_at(t0 - Duration::from_millis(5)), 0.0);
    }
}
