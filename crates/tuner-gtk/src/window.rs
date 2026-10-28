//! The main window. Layout: header (account menu, refresh button,
//! search, details toggle), groups sidebar,
//! channel list or cards, details panel, status bar. Blocking work
//! (loading the source and the movie / series catalogues, the stream-limit
//! check, starting mpv) runs on GIO worker threads so the UI never freezes.
//! Channel lists and catalogues arrive in batches while they download and are
//! appended to the open view as they come. Play/stop/limit logic is the
//! shared `tuner_app::Controller`, identical to the TUI.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib, pango};
use tuner_app::config::SourceKind;
use tuner_app::{
    Catalog, Config, Controller, Flow, Playback, Request, Resolved, Source, StreamGuard, Tick,
    status_line,
};
use tuner_core::{Channel, Entry, Library, Series};
use tuner_mpv::{LoadStage, MpvOptions, Player};

use crate::accounts;
use crate::logos::{Logo, Logos};
use crate::theme;
use crate::themes;

/// External mpv, or mpv embedded in the window (feature `embedded`).
type AnyPlayer = Box<dyn Playback + Send>;

/// The embedded video (feature `embedded`): its pane (video and control
/// bar) and how a stream moves to mpv's own window.
#[cfg_attr(not(feature = "embedded"), allow(dead_code))]
pub(crate) struct Video {
    pub(crate) widget: gtk::Widget,
    /// "Open in mpv window" on the control bar.
    pub(crate) detach: gtk::Button,
    /// Set by the player while the stream plays in mpv's own window.
    pub(crate) detached: Arc<AtomicBool>,
    /// Asks the player to play the next stream in mpv's own window.
    pub(crate) detach_next: Arc<AtomicBool>,
}
type Ctl = Arc<Mutex<Controller<AnyPlayer, StreamGuard>>>;

/// How often to ask the provider whether the account is over its limit while
/// something plays.
const WATCH_INTERVAL: Duration = Duration::from_secs(60);

/// How often batches of a downloading list are added to the view. Coalescing
/// them keeps each main-loop update small, so the UI stays fluid.
const BATCH_INTERVAL: Duration = Duration::from_millis(120);
/// Until the first batch arrives the main loop looks sooner, doubling from
/// this up to [`BATCH_INTERVAL`], so a list that loads fast shows at once.
const FIRST_BATCH_POLL: Duration = Duration::from_millis(4);

/// Fixed logo sizes and bounded sidebar widths. Artwork and long names must
/// not choose the layout width.
const DETAILS_LOGO: i32 = 120;
const DETAILS_WIDTH: i32 = 320;
const ROW_LOGO: i32 = 36;
const PLAYER_LOGO: i32 = 40;
const CARD_LOGO: i32 = 80;
const DETAILS_BREAKPOINT: i32 = 1000;
const GROUPS_MIN: i32 = 180;
const GROUPS_MAX: i32 = 360;
const DETAILS_MIN: i32 = 260;
const DETAILS_MAX: i32 = 440;
const BROWSER_MIN: i32 = 300;
/// Width of the channel list beside the embedded video.
const LIST_BESIDE_VIDEO: i32 = 360;
const HANDLE_WIDTH: i32 = 6;

/// Rows in the details grid. Each section fills as many as it has fields for
/// and hides the rest.
const FIELD_ROWS: usize = 7;

/// The player slot: the controller only needs to know that something plays.
/// The window tracks *what* itself, since it browses several lists.
const SLOT: usize = 0;

/// What the list shows: live TV, or the on-demand catalogues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Live,
    Movies,
    Series,
}

const SECTIONS: [(Section, &str); 3] = [
    (Section::Live, "Live"),
    (Section::Movies, "Movies"),
    (Section::Series, "Series"),
];

/// A series opened to its episodes, plus the sidebar row to return to.
struct Drill {
    episodes: Library,
    back_group: i32,
}

/// One list entry, borrowed from whichever library is shown.
enum Shown<'a> {
    Channel(&'a Channel),
    Series(&'a Series),
}

/// Words for the section on show.
struct Nouns {
    all: &'static str,
    one: &'static str,
    many: &'static str,
    heading: &'static str,
}

#[derive(Clone, Copy)]
struct ResizeDrag {
    details: bool,
    width: i32,
    origin: f64,
}

pub(crate) struct Widgets {
    pub(crate) window: gtk::ApplicationWindow,
    account_menu: gtk::MenuButton,
    account_popover: gtk::Popover,
    account_name: gtk::Label,
    account_kind: gtk::Label,
    account_list: gtk::Box,
    account_manage: gtk::Button,
    refresh: gtk::Button,
    sections: Vec<(Section, gtk::ToggleButton)>,
    back: gtk::Button,
    empty_add: gtk::Button,
    stack: gtk::Stack,
    loading_label: gtk::Label,
    error_label: gtk::Label,
    groups: gtk::ListBox,
    groups_heading: gtk::Label,
    channels_title: gtk::Label,
    channels_count: gtk::Label,
    /// Next to the count while the shown list is still downloading.
    list_spinner: gtk::Spinner,
    no_results_title: gtk::Label,
    no_results_hint: gtk::Label,
    results: gtk::Stack,
    body_size: gtk::WidgetPaintable,
    sidebar: gtk::Box,
    groups_handle: gtk::Box,
    details_handle: gtk::Box,
    search: gtk::SearchEntry,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    list: gtk::ListView,
    factory: gtk::SignalListItemFactory,
    cards: gtk::GridView,
    card_factory: gtk::SignalListItemFactory,
    views: gtk::Stack,
    list_toggle: gtk::ToggleButton,
    cards_toggle: gtk::ToggleButton,
    details_toggle: gtk::ToggleButton,
    themes: gtk::MenuButton,
    theme_list: gtk::ListBox,
    /// Check mark of each `theme_list` row, shown on the current theme.
    theme_checks: Vec<gtk::Image>,
    player: PlayerBar,
    details: Details,
    status: gtk::Label,
    summary: gtk::Label,
    /// The embedded video, shown while something plays in it; `None` when
    /// mpv always plays in its own window.
    video: Option<Video>,
}

/// Bottom bar shown while something plays.
struct PlayerBar {
    revealer: gtk::Revealer,
    logo: Logo,
    title: gtk::Label,
    subtitle: gtk::Label,
    state: gtk::Label,
    pause: gtk::Button,
    stop: gtk::Button,
}

struct Details {
    revealer: gtk::Revealer,
    logo: Logo,
    title: gtk::Label,
    subtitle: gtk::Label,
    names: Vec<gtk::Label>,
    values: Vec<gtk::Label>,
    play: gtk::Button,
    play_label: gtk::Label,
    /// Reveals the stream URL, which embeds the account password.
    reveal_url: gtk::ToggleButton,
}

/// What to show when the window opens.
pub struct Start {
    pub config: Config,
    /// Where account changes are saved. `None` disables saving.
    pub config_path: Option<PathBuf>,
    /// Source to open: its config name (`None` for a path/URL given on the
    /// command line) and how to open it. `None` shows the empty state.
    pub initial: Option<(Option<String>, Resolved)>,
    pub mpv: MpvOptions,
    pub theme: theme::Choice,
}

pub(crate) struct Shared {
    pub(crate) w: Widgets,
    /// Live channels. Movies and series are fetched the first time they are
    /// opened.
    live: RefCell<Library>,
    movies: RefCell<Option<Library>>,
    series: RefCell<Option<Library<Series>>>,
    drill: RefCell<Option<Drill>>,
    section: Cell<Section>,
    /// The open source, for fetching catalogues; `None` until it has loaded.
    catalog: RefCell<Option<Source>>,
    /// An episode request is running.
    vod_busy: Cell<bool>,
    /// Sections whose list is still downloading.
    streaming: RefCell<HashSet<Section>>,
    /// Sidebar rows: index 0 is "All", then the shown library's groups.
    group_names: RefCell<Vec<String>>,
    /// Count label of each sidebar row, updated as batches arrive.
    group_counts: RefCell<Vec<gtk::Label>>,
    controller: RefCell<Option<Ctl>>,
    pub(crate) config: RefCell<Config>,
    config_path: Option<PathBuf>,
    /// Config name of the open source; `None` for a command-line path/URL or
    /// when nothing is open.
    pub(crate) current: RefCell<Option<String>>,
    /// Makes the player for each opened source.
    new_player: Box<dyn Fn() -> AnyPlayer>,
    /// Bumped whenever the open source changes, so late results from a
    /// previous load are ignored.
    generation: Cell<u64>,
    /// Set while the sidebar selection is updated programmatically.
    syncing: Cell<bool>,
    /// What a command-line path/URL resolved to, so it can be refreshed;
    /// `None` for configured accounts (those are re-resolved from the config).
    adhoc: RefCell<Option<Resolved>>,
    /// Set while the sidebar and list are rebuilt, so the many selection and
    /// search changes along the way don't each refilter the whole library.
    rebuilding: Cell<bool>,
    /// Identifies the running fade-in, so a newer one cancels an older one.
    fade: Cell<u64>,
    /// What plays now, for rendering without locking the controller.
    now: RefCell<Option<Channel>>,
    /// Load stage last put in the status line, to update it only on change.
    stage: Cell<Option<LoadStage>>,
    paused: Cell<bool>,
    /// The details toggle as it was before the embedded video opened (which
    /// closes the panel); restored when the video closes.
    details_before_video: Cell<Option<bool>>,
    busy: Cell<bool>,
    groups_width: Cell<i32>,
    details_width: Cell<i32>,
    resize_drag: Cell<Option<ResizeDrag>>,
    logos: Logos,
    /// Playing-marker labels of currently bound rows, by (cards view, list index).
    markers: RefCell<HashMap<(bool, usize), glib::WeakRef<gtk::Label>>>,
    theme: theme::Live,
}

pub(crate) type S = Rc<Shared>;

impl Shared {
    fn controller(&self) -> Option<Ctl> {
        self.controller.borrow().clone()
    }

    fn is_playing(&self, channel: &Channel) -> bool {
        self.now
            .borrow()
            .as_ref()
            .is_some_and(|c| c.url == channel.url)
    }
}

pub fn build(app: &gtk::Application, start: Start) {
    let display = gtk::gdk::Display::default().expect("no display");
    let theme_file = start.config_path.as_deref().map(theme::file_beside);
    let theme = theme::install(&display, start.theme, theme_file);
    let (video, new_player) = players(start.mpv);

    let s: S = Rc::new(Shared {
        w: build_widgets(app, video),
        live: RefCell::default(),
        movies: RefCell::default(),
        series: RefCell::default(),
        drill: RefCell::default(),
        section: Cell::new(Section::Live),
        catalog: RefCell::default(),
        vod_busy: Cell::new(false),
        streaming: RefCell::default(),
        group_names: RefCell::default(),
        group_counts: RefCell::default(),
        controller: RefCell::new(None),
        config: RefCell::new(start.config),
        config_path: start.config_path,
        current: RefCell::new(None),
        new_player,
        generation: Cell::new(0),
        syncing: Cell::new(false),
        adhoc: RefCell::new(None),
        rebuilding: Cell::new(false),
        fade: Cell::new(0),
        now: RefCell::default(),
        stage: Cell::new(None),
        paused: Cell::new(false),
        details_before_video: Cell::new(None),
        busy: Cell::new(false),
        groups_width: Cell::new(200),
        details_width: Cell::new(DETAILS_WIDTH),
        resize_drag: Cell::new(None),
        logos: Logos::default(),
        markers: RefCell::default(),
        theme,
    });
    connect(&s);
    s.w.window.present();
    match start.initial {
        Some((name, resolved)) => open(&s, name, resolved),
        None => show_empty(&s),
    }
}

/// The embedded video and its players when built with `embedded` and
/// libmpv starts; otherwise an external mpv per source.
fn players(mpv: MpvOptions) -> (Option<Video>, Box<dyn Fn() -> AnyPlayer>) {
    #[cfg(feature = "embedded")]
    match crate::embed::start(&mpv) {
        Ok((video, make)) => return (Some(video), Box::new(make)),
        Err(e) => eprintln!("tuner-gtk: embedded video unavailable, using an mpv window: {e}"),
    }
    (None, Box::new(move || Box::new(Player::new(mpv.clone()))))
}

fn build_widgets(app: &gtk::Application, video: Option<Video>) -> Widgets {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Tuner")
        .default_width(1180)
        .default_height(740)
        .build();

    let header = gtk::HeaderBar::new();
    header.add_css_class("browser-header");
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search channels")
        .width_request(240)
        .hexpand(true)
        .build();
    header.set_title_widget(Some(&search));
    let brand = gtk::Label::new(Some("Tuner"));
    brand.add_css_class("brand");
    header.pack_start(&brand);
    let account_name = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .max_width_chars(18)
        .build();
    account_name.set_valign(gtk::Align::Center);
    account_name.add_css_class("account-name");
    let account_kind = gtk::Label::builder()
        .xalign(0.0)
        .valign(gtk::Align::Center)
        .build();
    account_kind.add_css_class("secondary-label");
    let account_text = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    account_text.append(&account_name);
    account_text.append(&account_kind);
    let account_content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    account_content.append(&account_text);
    account_content.append(&gtk::Image::from_icon_name("pan-down-symbolic"));

    let account_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let account_manage = popover_action("system-users-symbolic", "Add or manage accounts");
    let popover_heading = gtk::Label::builder()
        .label("MY PLAYLISTS")
        .xalign(0.0)
        .build();
    popover_heading.add_css_class("section-label");
    let popover_body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    popover_body.append(&popover_heading);
    popover_body.append(&account_list);
    popover_body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    popover_body.append(&account_manage);
    let account_popover = gtk::Popover::builder().child(&popover_body).build();
    let account_menu = gtk::MenuButton::builder()
        .child(&account_content)
        .popover(&account_popover)
        .tooltip_text("Playlists (Ctrl+O)")
        .build();
    account_menu.add_css_class("account-menu");
    let refresh = icon_button("view-refresh-symbolic", "Refresh playlist (Ctrl+R)");
    header.pack_start(&account_menu);
    header.pack_start(&refresh);
    let details_toggle = gtk::ToggleButton::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("Details (Ctrl+I)")
        .active(true)
        .build();
    header.pack_end(&details_toggle);
    let (themes, theme_list, theme_checks) = build_theme_menu();
    header.pack_end(&themes);
    header.connect_map(align_window_controls);
    window.set_titlebar(Some(&header));

    let groups = gtk::ListBox::new();
    groups.set_selection_mode(gtk::SelectionMode::Single);
    let groups_scroll = gtk::ScrolledWindow::builder()
        .child(&groups)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.set_width_request(GROUPS_MIN);
    // Explicitly not expanding: a child that wants to fill the sidebar must
    // not make it compete with the channel list for the window's width.
    sidebar.set_hexpand(false);
    sidebar.add_css_class("sidebar");
    // Section switch: the top of the sidebar, where the lists it changes are.
    let switcher = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    switcher.add_css_class("linked");
    switcher.add_css_class("section-switch");
    switcher.set_homogeneous(true);
    switcher.set_margin_top(12);
    switcher.set_margin_start(10);
    switcher.set_margin_end(10);
    switcher.set_margin_bottom(4);
    let mut sections: Vec<(Section, gtk::ToggleButton)> = Vec::new();
    for (section, label) in SECTIONS {
        let button = gtk::ToggleButton::with_label(label);
        if let Some((_, first)) = sections.first() {
            button.set_group(Some(first));
        }
        switcher.append(&button);
        sections.push((section, button));
    }
    sections[0].1.set_active(true);
    sidebar.append(&switcher);
    let groups_heading = section_label("Your channels");
    groups_heading.add_css_class("sidebar-heading");
    sidebar.append(&groups_heading);
    sidebar.append(&groups_scroll);

    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    let factory = gtk::SignalListItemFactory::new();
    let list = gtk::ListView::new(Some(selection.clone()), Some(factory.clone()));
    list.add_css_class("channel-list");
    let list_scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    let card_factory = gtk::SignalListItemFactory::new();
    let cards = gtk::GridView::new(Some(selection.clone()), Some(card_factory.clone()));
    cards.set_min_columns(1);
    cards.set_max_columns(12);
    cards.add_css_class("channel-cards");
    let cards_scroll = gtk::ScrolledWindow::builder()
        .child(&cards)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    let views = gtk::Stack::new();
    views.set_hhomogeneous(false);
    views.add_named(&list_scroll, Some("list"));
    views.add_named(&cards_scroll, Some("cards"));
    views.set_visible_child_name("list");
    let channels_title = gtk::Label::builder()
        .label("All channels")
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .hexpand(true)
        .build();
    channels_title.add_css_class("section-title");
    let channels_count = gtk::Label::new(Some("0 channels"));
    channels_count.add_css_class("secondary-label");
    let list_heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    list_heading.add_css_class("list-heading");
    // Leaving a series' episodes sits where the title is, not in the header.
    let back_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    back_content.append(&gtk::Image::from_icon_name("go-previous-symbolic"));
    back_content.append(&gtk::Label::new(Some("Series")));
    let back = gtk::Button::new();
    back.set_child(Some(&back_content));
    back.add_css_class("flat");
    back.set_tooltip_text(Some("Back to series (Alt+Left)"));
    back.update_property(&[gtk::accessible::Property::Label("Back to series")]);
    back.set_visible(false);
    list_heading.append(&back);
    list_heading.append(&channels_title);
    let list_spinner = gtk::Spinner::builder()
        .tooltip_text("Still loading")
        .visible(false)
        .build();
    list_heading.append(&list_spinner);
    list_heading.append(&channels_count);
    let list_toggle = gtk::ToggleButton::builder()
        .icon_name("view-list-symbolic")
        .tooltip_text("List view")
        .active(true)
        .build();
    list_toggle.update_property(&[gtk::accessible::Property::Label("List view")]);
    let cards_toggle = gtk::ToggleButton::builder()
        .icon_name("view-grid-symbolic")
        .tooltip_text("Cards view")
        .group(&list_toggle)
        .build();
    cards_toggle.update_property(&[gtk::accessible::Property::Label("Cards view")]);
    let view_controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    view_controls.add_css_class("linked");
    view_controls.add_css_class("view-controls");
    view_controls.append(&list_toggle);
    view_controls.append(&cards_toggle);
    list_heading.append(&view_controls);
    let no_results = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let no_results_title = gtk::Label::new(Some("No matching channels"));
    no_results_title.add_css_class("section-title");
    let no_results_hint = gtk::Label::builder()
        .label("Try another search or channel group.")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    no_results_hint.add_css_class("secondary-label");
    no_results.append(&no_results_title);
    no_results.append(&no_results_hint);
    let results = gtk::Stack::new();
    results.set_vexpand(true);
    results.add_named(&views, Some("content"));
    results.add_named(&no_results, Some("empty"));
    let channels = gtk::Box::new(gtk::Orientation::Vertical, 0);
    channels.append(&list_heading);
    channels.append(&results);

    let loading_label = gtk::Label::builder()
        .label("Loading…")
        .wrap(true)
        .max_width_chars(36)
        .justify(gtk::Justification::Center)
        .build();
    let loading = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    loading.append(
        &gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .build(),
    );
    loading.append(&loading_label);

    let error_label = gtk::Label::builder()
        .wrap(true)
        .selectable(true)
        .justify(gtk::Justification::Center)
        .valign(gtk::Align::Center)
        .margin_start(48)
        .margin_end(48)
        .build();
    error_label.add_css_class("error");

    let empty = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    let empty_title = gtk::Label::new(Some("No accounts yet"));
    empty_title.add_css_class("details-title");
    let empty_text = gtk::Label::builder()
        .label("Add an Xtream Codes login or an M3U playlist to start watching.")
        .wrap(true)
        .max_width_chars(36)
        .justify(gtk::Justification::Center)
        .build();
    empty_text.add_css_class("dim-label");
    let empty_add = gtk::Button::with_label("Add account");
    empty_add.add_css_class("suggested-action");
    empty_add.set_halign(gtk::Align::Center);
    empty.append(&empty_title);
    empty.append(&empty_text);
    empty.append(&empty_add);

    let stack = gtk::Stack::new();
    // Hidden welcome/loading text must not set the channel browser's minimum
    // width, otherwise a tiled window cannot reach the details breakpoint.
    stack.set_hhomogeneous(false);
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&channels, Some("channels"));
    stack.add_named(&error_label, Some("error"));
    stack.set_visible_child_name("loading");
    stack.set_hexpand(true);
    stack.set_width_request(BROWSER_MIN);

    let details = build_details();

    let groups_handle = resize_handle("Resize channel groups");
    let details_handle = resize_handle("Resize channel details");
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    body.append(&sidebar);
    body.append(&groups_handle);
    match &video {
        // Channel list left of the video, which takes the window's extra
        // width; the divider is draggable.
        Some(video) => {
            let center = gtk::Paned::builder()
                .start_child(&stack)
                .end_child(&video.widget)
                .shrink_start_child(false)
                .shrink_end_child(false)
                .resize_start_child(false)
                .position(LIST_BESIDE_VIDEO)
                .hexpand(true)
                .build();
            body.append(&center);
        }
        None => body.append(&stack),
    }
    body.append(&details_handle);
    body.append(&details.revealer);
    // Observe the available viewport rather than the children's requested
    // widths, so wide sidebar preferences cannot prevent narrowing a window.
    // The layout below always fits its viewport; no scrollbar is presented.
    let body_viewport = gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .min_content_width(GROUPS_MIN + BROWSER_MIN + HANDLE_WIDTH)
        .vexpand(true)
        .build();
    let body_size = gtk::WidgetPaintable::new(Some(&body_viewport));

    let player = build_player_bar();
    let status = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    let summary = gtk::Label::builder()
        .ellipsize(pango::EllipsizeMode::Start)
        .build();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    bar.add_css_class("statusbar");
    bar.append(&status);
    bar.append(&summary);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&body_viewport);
    root.append(&player.revealer);
    root.append(&bar);
    window.set_child(Some(&root));

    Widgets {
        window,
        account_menu,
        account_popover,
        account_name,
        account_kind,
        account_list,
        account_manage,
        refresh,
        sections,
        back,
        empty_add,
        stack,
        loading_label,
        error_label,
        groups,
        groups_heading,
        channels_title,
        channels_count,
        list_spinner,
        no_results_title,
        no_results_hint,
        results,
        body_size,
        sidebar,
        groups_handle,
        details_handle,
        search,
        store,
        selection,
        list,
        factory,
        cards,
        card_factory,
        views,
        list_toggle,
        cards_toggle,
        details_toggle,
        themes,
        theme_list,
        theme_checks,
        player,
        details,
        status,
        summary,
        video,
    }
}

fn build_details() -> Details {
    // Fixed pixel size: a large logo never widens the panel.
    let logo = Logo::new(DETAILS_LOGO);
    logo.widget.set_margin_top(20);
    logo.widget.set_margin_bottom(20);
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("logo-frame");
    frame.append(&logo.widget);

    let title = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .max_width_chars(24)
        .xalign(0.0)
        .build();
    title.add_css_class("details-title");
    let subtitle = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .max_width_chars(24)
        .xalign(0.0)
        .build();
    subtitle.add_css_class("secondary-label");
    let identity = gtk::Box::new(gtk::Orientation::Vertical, 6);
    identity.append(&title);
    identity.append(&subtitle);

    let grid = gtk::Grid::builder()
        .row_spacing(14)
        .column_spacing(14)
        .build();
    let mut names = Vec::new();
    let mut values = Vec::new();
    for row in 0..FIELD_ROWS as i32 {
        let label = gtk::Label::builder().xalign(0.0).build();
        label.add_css_class("field-label");
        let value = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .max_width_chars(18)
            .selectable(true)
            .build();
        grid.attach(&label, 0, row, 1, 1);
        grid.attach(&value, 1, row, 1, 1);
        names.push(label);
        values.push(value);
    }

    let play = gtk::Button::new();
    let play_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    play_content.append(&gtk::Image::from_icon_name("media-playback-start-symbolic"));
    let play_label = gtk::Label::new(Some("Play channel"));
    play_content.append(&play_label);
    play.set_child(Some(&play_content));
    play.set_tooltip_text(Some("Play selected channel (Enter)"));
    play.update_property(&[gtk::accessible::Property::Label("Play channel")]);
    play.add_css_class("suggested-action");
    play.add_css_class("channel-play");

    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    panel.append(&section_label("Selected channel"));
    panel.append(&frame);
    panel.append(&identity);
    panel.append(&play);
    panel.append(&grid);
    let reveal_url = gtk::ToggleButton::with_label("Show stream URL");
    reveal_url.set_tooltip_text(Some("The URL usually contains your account password"));
    reveal_url.set_halign(gtk::Align::Start);
    panel.append(&reveal_url);
    // Constrain content to the user-sized pane instead of letting long names
    // or artwork choose the sidebar width.
    let outer = gtk::ScrolledWindow::builder()
        .child(&panel)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .width_request(DETAILS_MIN)
        .build();
    // Explicitly not expanding, so no child can make the panel grab space
    // from the channel list.
    outer.set_hexpand(false);
    outer.add_css_class("details");

    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::None)
        .reveal_child(true)
        .child(&outer)
        .build();
    Details {
        revealer,
        logo,
        title,
        subtitle,
        names,
        values,
        play,
        play_label,
        reveal_url,
    }
}

fn build_player_bar() -> PlayerBar {
    let logo = Logo::new(PLAYER_LOGO);
    let title = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    title.add_css_class("player-title");
    let subtitle = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    subtitle.add_css_class("dim-label");
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    text.append(&title);
    text.append(&subtitle);

    let pause = icon_button("media-playback-pause-symbolic", "Pause (Ctrl+P)");
    let stop = icon_button("media-playback-stop-symbolic", "Stop (Ctrl+S)");
    for button in [&pause, &stop] {
        button.add_css_class("circular");
        button.set_valign(gtk::Align::Center);
    }

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.add_css_class("player-bar");
    bar.append(&logo_tile(&logo));
    bar.append(&text);
    let state = gtk::Label::new(Some("Playing in mpv"));
    state.add_css_class("playback-state");
    bar.append(&state);
    bar.append(&pause);
    bar.append(&stop);

    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(false)
        .child(&bar)
        .build();
    PlayerBar {
        revealer,
        logo,
        title,
        subtitle,
        state,
        pause,
        stop,
    }
}

/// Show the pause or play icon on the bar's button.
fn set_paused(s: &S, paused: bool) {
    let changed = s.paused.replace(paused) != paused;
    let (icon, tip) = if paused {
        ("media-playback-start-symbolic", "Resume (Ctrl+P)")
    } else {
        ("media-playback-pause-symbolic", "Pause (Ctrl+P)")
    };
    s.w.player.pause.set_icon_name(icon);
    s.w.player.pause.set_tooltip_text(Some(tip));
    s.w.player
        .state
        .set_text(if paused { "Paused" } else { "Playing in mpv" });
    if changed {
        update_playing_markers(s);
    }
}

fn section_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    label.add_css_class("section-label");
    label
}

fn logo_tile(logo: &Logo) -> gtk::Box {
    let tile = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    tile.add_css_class("logo-tile");
    tile.append(&logo.widget);
    tile
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build()
}

/// Header button with the theme list. Row 0 is the theme file, row `n` is
/// `themes::ALL[n - 1]`. Also returns each row's check mark.
fn build_theme_menu() -> (gtk::MenuButton, gtk::ListBox, Vec<gtk::Image>) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    let mut checks = Vec::new();
    let labels = std::iter::once("Custom (theme.toml)").chain(themes::ALL.iter().map(|t| t.label));
    for label in labels {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let name = gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .build();
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.set_opacity(0.0);
        row.append(&name);
        row.append(&check);
        list.append(&row);
        checks.push(check);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(420)
        .build();
    let popover = gtk::Popover::new();
    popover.set_child(Some(&scroll));
    let button = gtk::MenuButton::builder()
        .icon_name("preferences-color-symbolic")
        .tooltip_text("Theme (Ctrl+T)")
        .popover(&popover)
        .build();
    button.add_css_class("theme-picker");
    (button, list, checks)
}

/// The theme in row `index` of the theme list.
fn choice_for_row(index: usize) -> theme::Choice {
    match index.checked_sub(1).and_then(|i| themes::ALL.get(i)) {
        Some(theme) => theme::Choice::Builtin(theme),
        None => theme::Choice::Custom,
    }
}

/// Row of the theme list for `choice`.
fn theme_row(choice: theme::Choice) -> usize {
    match choice {
        theme::Choice::Custom => 0,
        theme::Choice::Builtin(t) => themes::ALL
            .iter()
            .position(|b| b.name == t.name)
            .map_or(0, |i| i + 1),
    }
}

/// Check the current theme's row, and put the cursor on it.
fn sync_theme_menu(s: &S) {
    let current = theme_row(s.theme.choice());
    for (i, check) in s.w.theme_checks.iter().enumerate() {
        check.set_opacity(if i == current { 1.0 } else { 0.0 });
    }
    if let Some(row) = s.w.theme_list.row_at_index(current as i32) {
        s.w.theme_list.select_row(Some(&row));
    }
}

/// Apply the theme in row `index` of the theme list and remember it in the
/// config. The theme file is the default, so it is stored as no setting.
fn choose_theme(s: &S, index: usize) {
    let choice = choice_for_row(index);
    s.theme.set(choice);
    s.w.themes.popdown();
    sync_theme_menu(s);
    let name = match choice {
        theme::Choice::Custom => None,
        theme::Choice::Builtin(t) => Some(t.name.to_string()),
    };
    let changed = s.config.borrow().theme != name;
    if !changed {
        return;
    }
    s.config.borrow_mut().theme = name;
    match save_config(s) {
        Ok(()) => set_status(s, "Theme saved"),
        Err(e) => set_status(s, &format!("Theme changed, but not saved: {e}")),
    }
}

fn align_window_controls(widget: &impl IsA<gtk::Widget>) {
    let widget = widget.as_ref();
    if widget.is::<gtk::WindowControls>() {
        widget.set_valign(gtk::Align::Center);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        align_window_controls(&current);
        child = current.next_sibling();
    }
}

fn resize_handle(label: &str) -> gtk::Box {
    let handle = gtk::Box::new(gtk::Orientation::Vertical, 0);
    handle.add_css_class("resize-handle");
    handle.set_width_request(HANDLE_WIDTH);
    handle.set_vexpand(true);
    handle.set_cursor_from_name(Some("col-resize"));
    handle.set_focusable(true);
    handle.set_can_target(true);
    handle.set_tooltip_text(Some("Drag to resize. Use arrow keys when focused."));
    handle.update_property(&[gtk::accessible::Property::Label(label)]);
    handle
}

fn connect_resize_handle(s: &S, handle: &gtk::Box, details: bool) {
    let press = gtk::GestureClick::new();
    press.set_button(gtk::gdk::BUTTON_PRIMARY);
    let weak = Rc::downgrade(s);
    let handle_weak = handle.downgrade();
    press.connect_pressed(move |gesture, _, _, _| {
        if let Some(s) = weak.upgrade()
            && let Some((origin, _)) = gesture.current_event().and_then(|event| event.position())
        {
            s.resize_drag.set(Some(ResizeDrag {
                details,
                width: sidebar_width(&s, details),
                origin,
            }));
            if let Some(handle) = handle_weak.upgrade() {
                handle.grab_focus();
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    handle.add_controller(press);

    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(s);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(s) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        use gtk::gdk::Key;
        let current = sidebar_width(&s, details);
        let step = if details { -10 } else { 10 };
        let width = match key {
            Key::Left => current - step,
            Key::Right => current + step,
            Key::Home => {
                if details {
                    DETAILS_MIN
                } else {
                    GROUPS_MIN
                }
            }
            Key::End => {
                if details {
                    DETAILS_MAX
                } else {
                    GROUPS_MAX
                }
            }
            _ => return glib::Propagation::Proceed,
        };
        resize_sidebar(&s, details, width);
        glib::Propagation::Stop
    });
    handle.add_controller(keys);
}

fn sidebar_width(s: &S, details: bool) -> i32 {
    if details {
        s.w.details
            .revealer
            .child()
            .map_or(DETAILS_WIDTH, |pane| pane.width())
    } else {
        s.w.sidebar.width()
    }
}

fn resize_sidebar(s: &S, details: bool, width: i32) {
    if details {
        s.details_width.set(width.clamp(DETAILS_MIN, DETAILS_MAX));
    } else {
        s.groups_width.set(width.clamp(GROUPS_MIN, GROUPS_MAX));
    }
    update_details_visibility(s);
}

/// A flat, left-aligned button with an icon, for the account popover.
fn popover_action(icon: &str, label: &str) -> gtk::Button {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&gtk::Image::from_icon_name(icon));
    row.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder().child(&row).build();
    button.add_css_class("flat");
    button.add_css_class("account-action");
    button
}

/// A row in the account popover: name, then the account type.
fn account_row(name: &str, kind: &str, current: bool) -> gtk::Button {
    let title = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .max_width_chars(36)
        .build();
    let subtitle = gtk::Label::builder().label(kind).xalign(0.0).build();
    subtitle.add_css_class("secondary-label");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.append(&title);
    text.append(&subtitle);
    let button = gtk::Button::builder().child(&text).build();
    button.add_css_class("flat");
    button.add_css_class("account-row");
    if current {
        button.add_css_class("current");
    }
    button
}

fn connect_channel_factory(s: &S, factory: &gtk::SignalListItemFactory, cards: bool) {
    factory.connect_setup(move |_, obj| {
        let item = obj.downcast_ref::<gtk::ListItem>().expect("ListItem");
        let row = gtk::Box::new(
            if cards {
                gtk::Orientation::Vertical
            } else {
                gtk::Orientation::Horizontal
            },
            if cards { 10 } else { 12 },
        );
        row.add_css_class(if cards { "channel-card" } else { "channel-row" });
        if cards {
            row.set_width_request(150);
        }
        let marker = gtk::Label::builder()
            .width_chars(9)
            .xalign(if cards { 0.5 } else { 1.0 })
            .build();
        if cards {
            marker.set_height_request(18);
        }
        marker.add_css_class("playing");
        let number = gtk::Label::builder().width_chars(3).xalign(1.0).build();
        number.set_visible(!cards);
        number.add_css_class("channel-number");
        let logo = Logo::new(if cards { CARD_LOGO } else { ROW_LOGO });
        let name = gtk::Label::builder()
            .xalign(if cards { 0.5 } else { 0.0 })
            .ellipsize(pango::EllipsizeMode::End)
            .wrap(cards)
            .wrap_mode(pango::WrapMode::WordChar)
            .lines(if cards { 2 } else { 1 })
            .max_width_chars(if cards { 20 } else { -1 })
            .justify(if cards {
                gtk::Justification::Center
            } else {
                gtk::Justification::Left
            })
            .build();
        name.add_css_class("channel-name");
        let subtitle = gtk::Label::builder()
            .xalign(if cards { 0.5 } else { 0.0 })
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        subtitle.add_css_class("channel-subtitle");
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(3)
            .valign(gtk::Align::Center)
            .hexpand(!cards)
            .build();
        if cards {
            text.set_height_request(60);
        }
        text.append(&name);
        text.append(&subtitle);
        let tile = logo_tile(&logo);
        for child in [
            number.upcast_ref::<gtk::Widget>(),
            tile.upcast_ref(),
            text.upcast_ref(),
            marker.upcast_ref(),
        ] {
            row.append(child);
        }
        item.set_child(Some(&row));
    });

    let weak = Rc::downgrade(s);
    factory.connect_bind(move |_, obj| {
        let Some(s) = weak.upgrade() else { return };
        let item = obj.downcast_ref::<gtk::ListItem>().expect("ListItem");
        let Some(index) = item_index(item.item()) else {
            return;
        };
        let row = item.child().and_downcast::<gtk::Box>().expect("row");
        let number = row
            .first_child()
            .and_downcast::<gtk::Label>()
            .expect("number");
        let tile = number
            .next_sibling()
            .and_downcast::<gtk::Box>()
            .expect("logo tile");
        let logo = Logo::from_widget(
            tile.first_child()
                .and_downcast::<gtk::Overlay>()
                .expect("logo"),
        );
        let text = tile
            .next_sibling()
            .and_downcast::<gtk::Box>()
            .expect("channel text");
        let name = text
            .first_child()
            .and_downcast::<gtk::Label>()
            .expect("name");
        let subtitle = name
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .expect("subtitle");
        let marker = text
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .expect("marker");

        number.set_visible(!cards && s.section.get() == Section::Live);
        marker.set_text(marker_text(&s, index));
        with_entry(&s, index, |entry| match entry {
            Shown::Channel(c) => {
                number.set_text(&c.number.map(|n| n.to_string()).unwrap_or_default());
                name.set_text(&c.name);
                name.set_tooltip_text(Some(&c.name));
                let mut metadata = vec![c.group.as_deref().unwrap_or("Ungrouped").to_string()];
                if let Some(days) = c.catchup_days {
                    metadata.push(format!("{days}d replay"));
                }
                subtitle.set_text(&metadata.join(" · "));
                s.logos.show(c.logo.as_deref(), &c.name, &logo);
            }
            Shown::Series(series) => {
                number.set_text("");
                name.set_text(&series.name);
                name.set_tooltip_text(Some(&series.name));
                let mut metadata = vec![
                    series
                        .group
                        .as_deref()
                        .unwrap_or("Uncategorised")
                        .to_string(),
                ];
                let info = series_info(series);
                if !info.is_empty() {
                    metadata.push(info);
                }
                subtitle.set_text(&metadata.join(" · "));
                s.logos.show(series.cover.as_deref(), &series.name, &logo);
            }
        });
        s.markers
            .borrow_mut()
            .insert((cards, index), marker.downgrade());
    });

    let weak = Rc::downgrade(s);
    factory.connect_unbind(move |_, obj| {
        let Some(s) = weak.upgrade() else { return };
        let item = obj.downcast_ref::<gtk::ListItem>().expect("ListItem");
        if let Some(index) = item_index(item.item()) {
            let marker = item
                .child()
                .and_downcast::<gtk::Box>()
                .and_then(|row| row.last_child().and_downcast::<gtk::Label>());
            let mut markers = s.markers.borrow_mut();
            // Filtering can bind the replacement before unbinding the old
            // row. Only remove the marker owned by this particular item.
            if markers
                .get(&(cards, index))
                .and_then(glib::WeakRef::upgrade)
                == marker
            {
                markers.remove(&(cards, index));
            }
        }
    });
}

fn connect(s: &S) {
    let w = &s.w;
    // Observe the whole surface while a divider is held: it moves under the
    // pointer, so local widget coordinates cannot describe a stable drag.
    let resize_events = gtk::EventControllerLegacy::new();
    resize_events.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(s);
    resize_events.connect_event(move |_, event| {
        let Some(s) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let Some(drag) = s.resize_drag.get() else {
            return glib::Propagation::Proceed;
        };
        use gtk::gdk::EventType;
        let finished = matches!(
            event.event_type(),
            EventType::ButtonRelease | EventType::TouchEnd | EventType::TouchCancel
        );
        if finished
            || matches!(
                event.event_type(),
                EventType::MotionNotify | EventType::TouchUpdate
            )
        {
            if let Some((x, _)) = event.position() {
                let delta = (x - drag.origin) * if drag.details { -1.0 } else { 1.0 };
                resize_sidebar(&s, drag.details, drag.width + delta.round() as i32);
            }
            if finished {
                s.resize_drag.set(None);
            }
            return if finished {
                glib::Propagation::Proceed
            } else {
                glib::Propagation::Stop
            };
        }
        glib::Propagation::Proceed
    });
    w.window.add_controller(resize_events);
    let weak = Rc::downgrade(s);
    w.window.connect_is_active_notify(move |window| {
        if !window.is_active()
            && let Some(s) = weak.upgrade()
        {
            s.resize_drag.set(None);
        }
    });
    connect_channel_factory(s, &w.factory, false);
    connect_channel_factory(s, &w.card_factory, true);
    connect_resize_handle(s, &w.groups_handle, false);
    connect_resize_handle(s, &w.details_handle, true);

    for (toggle, name) in [(&w.list_toggle, "list"), (&w.cards_toggle, "cards")] {
        let weak = Rc::downgrade(s);
        toggle.connect_toggled(move |toggle| {
            if toggle.is_active()
                && let Some(s) = weak.upgrade()
            {
                s.w.views.set_visible_child_name(name);
                focus_channels(&s);
            }
        });
    }

    let weak = Rc::downgrade(s);
    w.theme_list.connect_row_activated(move |_, row| {
        if let Some(s) = weak.upgrade() {
            choose_theme(&s, row.index() as usize);
        }
    });
    if let Some(popover) = w.themes.popover() {
        let weak = Rc::downgrade(s);
        popover.connect_show(move |_| {
            if let Some(s) = weak.upgrade() {
                sync_theme_menu(&s);
                if let Some(row) = s.w.theme_list.selected_row() {
                    row.grab_focus();
                }
            }
        });
    }
    sync_theme_menu(s);

    let weak = Rc::downgrade(s);
    w.groups.connect_row_selected(move |_, _| {
        if let Some(s) = weak.upgrade()
            && !s.rebuilding.get()
        {
            refilter(&s);
        }
    });

    let weak = Rc::downgrade(s);
    w.search.connect_search_changed(move |_| {
        if let Some(s) = weak.upgrade()
            && !s.rebuilding.get()
        {
            refilter(&s);
        }
    });
    // Typing anywhere in the window goes to the search box.
    w.search.set_key_capture_widget(Some(&w.window));

    let weak = Rc::downgrade(s);
    w.search.connect_activate(move |_| {
        if let Some(s) = weak.upgrade()
            && let Some(index) = selected_index(&s)
        {
            activate(&s, index);
        }
    });

    let weak = Rc::downgrade(s);
    w.selection.connect_selected_item_notify(move |_| {
        if let Some(s) = weak.upgrade() {
            update_details(&s);
        }
    });

    let weak = Rc::downgrade(s);
    w.list.connect_activate(move |_, position| {
        if let Some(s) = weak.upgrade()
            && let Some(index) = item_index(s.w.store.item(position))
        {
            activate(&s, index);
        }
    });
    let weak = Rc::downgrade(s);
    w.cards.connect_activate(move |_, position| {
        if let Some(s) = weak.upgrade()
            && let Some(index) = item_index(s.w.store.item(position))
        {
            activate(&s, index);
        }
    });

    let weak = Rc::downgrade(s);
    w.details.reveal_url.connect_toggled(move |_| {
        if let Some(s) = weak.upgrade() {
            let button = &s.w.details.reveal_url;
            button.set_label(if button.is_active() {
                "Hide stream URL"
            } else {
                "Show stream URL"
            });
            update_details(&s);
        }
    });

    let weak = Rc::downgrade(s);
    w.details.play.connect_clicked(move |_| {
        if let Some(s) = weak.upgrade()
            && let Some(index) = selected_index(&s)
        {
            activate(&s, index);
        }
    });

    for (section, button) in &w.sections {
        let weak = Rc::downgrade(s);
        let section = *section;
        button.connect_toggled(move |b| {
            if let Some(s) = weak.upgrade()
                && b.is_active()
                && !s.syncing.get()
            {
                switch_section(&s, section);
            }
        });
    }
    let weak = Rc::downgrade(s);
    w.back.connect_clicked(move |_| {
        if let Some(s) = weak.upgrade() {
            close_series(&s);
        }
    });

    let weak = Rc::downgrade(s);
    w.player.pause.connect_clicked(move |_| {
        if let Some(s) = weak.upgrade() {
            toggle_pause(&s);
        }
    });
    let weak = Rc::downgrade(s);
    w.player.stop.connect_clicked(move |_| {
        if let Some(s) = weak.upgrade() {
            stop(&s);
        }
    });
    if let Some(video) = &w.video {
        let weak = Rc::downgrade(s);
        video.detach.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                detach(&s);
            }
        });
    }

    let weak = Rc::downgrade(s);
    w.details_toggle.connect_toggled(move |_| {
        if let Some(s) = weak.upgrade() {
            update_details_visibility(&s);
        }
    });
    // WidgetPaintable reports allocation changes without polling every frame.
    // Keep the requested toggle state, so widening restores the user's choice.
    let weak = Rc::downgrade(s);
    w.body_size.connect_invalidate_size(move |_| {
        if let Some(s) = weak.upgrade() {
            update_details_visibility(&s);
        }
    });
    let weak = Rc::downgrade(s);
    w.stack.connect_visible_child_name_notify(move |_| {
        if let Some(s) = weak.upgrade() {
            update_details_visibility(&s);
        }
    });

    let weak = Rc::downgrade(s);
    w.account_manage.connect_clicked(move |_| {
        if let Some(s) = weak.upgrade() {
            s.w.account_popover.popdown();
            accounts::open(&s, false);
        }
    });
    let weak = Rc::downgrade(s);
    w.refresh.connect_clicked(move |_| {
        if let Some(s) = weak.upgrade() {
            refresh(&s);
        }
    });
    let weak = Rc::downgrade(s);
    w.empty_add.connect_clicked(move |_| {
        if let Some(s) = weak.upgrade() {
            accounts::open(&s, true);
        }
    });

    install_shortcuts(s);

    // The window owns the shared state: this handler holds the only strong
    // reference that lives as long as the window. Every other handler holds a
    // `Weak`, so without this one the state is freed once loading finishes
    // and all input is silently ignored. On close it also quits mpv and
    // releases the stream slot.
    let owner = RefCell::new(Some(s.clone()));
    w.window.connect_close_request(move |_| {
        if let Some(s) = owner.borrow_mut().take()
            && let Some(ctl) = s.controller()
        {
            ctl.lock().unwrap_or_else(|e| e.into_inner()).shutdown();
        }
        glib::Propagation::Proceed
    });

    // While playing, ask the provider once a minute whether another device
    // pushed the account over its limit.
    let weak: Weak<Shared> = Rc::downgrade(s);
    glib::timeout_add_local(WATCH_INTERVAL, move || {
        let Some(s) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if let Some(ctl) = s.controller()
            && s.now.borrow().is_some()
        {
            glib::spawn_future_local(async move {
                if let Some(Some(warning)) = with_controller(&ctl, |c| c.watch()).await {
                    set_status(&s, "⚠ Account over its stream limit");
                    inform(&s, "Stream limit exceeded", &warning).await;
                }
            });
        }
        glib::ControlFlow::Continue
    });

    // SIGTERM/SIGINT/SIGHUP would otherwise kill us without the close
    // handler running, leaving mpv streaming on one of the account's
    // connections. The flag is polled by the timer below, which closes the
    // window (quitting mpv) instead.
    let terminate = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ] {
        let _ = signal_hook::flag::register(signal, terminate.clone());
    }

    // Notice when the user closes the mpv window or the stream fails.
    let weak: Weak<Shared> = Rc::downgrade(s);
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let Some(s) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if terminate.load(Ordering::Relaxed) {
            s.w.window.close();
            return glib::ControlFlow::Break;
        }
        // Never block the UI: skip this tick if a play/stop is in progress.
        let state = s.controller().and_then(|ctl| {
            ctl.try_lock()
                .ok()
                .map(|mut c| (c.tick(), c.is_paused(), c.progress()))
        });
        match state {
            Some((Some(tick), _, _)) => {
                let name = s.now.borrow().as_ref().map(|c| c.name.clone());
                let name = name.unwrap_or_default();
                set_playing(&s, None);
                match tick {
                    Tick::Closed => set_status(&s, "Player closed"),
                    Tick::Ended => set_status(&s, &format!("{name} ended")),
                    Tick::Failed(reason) => {
                        set_status(&s, &format!("Could not play {name}"));
                        let message = format!("{name}: {reason}. The player was closed.");
                        let s = s.clone();
                        glib::spawn_future_local(async move {
                            inform(&s, "Stream unavailable", &message).await;
                        });
                    }
                }
            }
            Some((None, paused, progress)) => {
                // Follow pauses made inside mpv (space in its window).
                if let Some(paused) = paused {
                    set_paused(&s, paused);
                }
                // Step, progress and elapsed time until the first frame (mpv
                // has no window before), then "Playing" once.
                let stage = progress.as_ref().map(|p| p.stage);
                let opening = stage.is_some_and(|st| st != LoadStage::Playing);
                let name = s.now.borrow().as_ref().map(|c| c.name.clone());
                if let Some(name) = name
                    && (opening || stage != s.stage.get())
                {
                    s.stage.set(stage);
                    set_status(&s, &status_line(&name, progress.as_ref()));
                }
            }
            None => {}
        }
        glib::ControlFlow::Continue
    });
}

fn install_shortcuts(s: &S) {
    let controller = gtk::ShortcutController::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let add = |trigger: &str, f: Box<dyn Fn(&S)>| {
        let weak = Rc::downgrade(s);
        let action = gtk::CallbackAction::new(move |_, _| {
            if let Some(s) = weak.upgrade() {
                f(&s);
            }
            glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(trigger),
            Some(action),
        ));
    };
    add(
        "<Control>f",
        Box::new(|s| {
            s.w.search.grab_focus();
        }),
    );
    add("<Control>p", Box::new(toggle_pause));
    add("<Control>s", Box::new(stop));
    add(
        "<Control>i",
        Box::new(|s| {
            let t = &s.w.details_toggle;
            t.set_active(!t.is_active());
        }),
    );
    add("<Control>q", Box::new(|s| s.w.window.close()));
    add("<Control>t", Box::new(|s| s.w.themes.popup()));
    add("<Control>1", Box::new(|s| switch_section(s, Section::Live)));
    add(
        "<Control>2",
        Box::new(|s| switch_section(s, Section::Movies)),
    );
    add(
        "<Control>3",
        Box::new(|s| switch_section(s, Section::Series)),
    );
    add("<Alt>Left", Box::new(close_series));
    add("<Control>comma", Box::new(|s| accounts::open(s, false)));
    add("<Control>o", Box::new(|s| s.w.account_menu.popup()));
    add("<Control>r", Box::new(refresh));
    add("F5", Box::new(refresh));
    s.w.window.add_controller(controller);
}

/// Switch to a source: stop and release the current one, then load.
pub(crate) fn open(s: &S, name: Option<String>, resolved: Resolved) {
    reset(s);
    *s.adhoc.borrow_mut() = name.is_none().then(|| resolved.clone());
    *s.current.borrow_mut() = name;
    sync_sources(s);
    load(s, resolved);
}

/// Open the configured account `name`. Runs its `password_command`, if any.
pub(crate) fn open_named(s: &S, name: &str) {
    let resolved = s.config.borrow().source(name).map(|c| c.resolve());
    match resolved {
        Some(Ok(resolved)) => open(s, Some(name.to_string()), resolved),
        Some(Err(e)) => {
            reset(s);
            *s.current.borrow_mut() = Some(name.to_string());
            sync_sources(s);
            show_error(s, &format!("Could not open {name}\n\n{e:#}"));
        }
        None => {}
    }
}

/// Reload the open playlist from its provider. Like switching account, this
/// stops playback and frees the stream slot first, so the limit still holds.
fn refresh(s: &S) {
    let name = s.current.borrow().clone();
    match name {
        Some(name) => open_named(s, &name),
        None => {
            let resolved = s.adhoc.borrow().clone();
            if let Some(resolved) = resolved {
                open(s, None, resolved);
            }
        }
    }
}

/// Nothing open: offer to add an account.
pub(crate) fn show_empty(s: &S) {
    reset(s);
    *s.adhoc.borrow_mut() = None;
    *s.current.borrow_mut() = None;
    sync_sources(s);
    s.w.window.set_title(Some("Tuner"));
    s.w.stack.set_visible_child_name("empty");
}

/// Stop playback (quitting mpv and releasing the stream slot) and clear the
/// channel list. Late results from an earlier load are ignored afterwards.
fn reset(s: &S) {
    s.generation.set(s.generation.get() + 1);
    if let Some(old) = s.controller.borrow_mut().take() {
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || {
                old.lock().unwrap_or_else(|e| e.into_inner()).shutdown();
            })
            .await;
        });
    }
    set_playing(s, None);
    s.busy.set(false);
    s.vod_busy.set(false);
    s.streaming.borrow_mut().clear();
    s.w.store.remove_all();
    *s.live.borrow_mut() = Library::default();
    *s.movies.borrow_mut() = None;
    *s.series.borrow_mut() = None;
    *s.drill.borrow_mut() = None;
    *s.catalog.borrow_mut() = None;
    s.section.set(Section::Live);
    s.rebuilding.set(true);
    while let Some(child) = s.w.groups.first_child() {
        s.w.groups.remove(&child);
    }
    s.rebuilding.set(false);
    s.w.summary.set_text("");
    s.w.channels_title.set_text("All channels");
    s.w.channels_count.set_text("0 channels");
    set_status(s, "");
    sync_chrome(s);
    update_details(s);
}

fn show_error(s: &S, text: &str) {
    s.w.error_label.set_text(text);
    s.w.stack.set_visible_child_name("error");
}

/// Refill the account menu from the config and mark the open account.
pub(crate) fn sync_sources(s: &S) {
    let w = &s.w;
    while let Some(child) = w.account_list.first_child() {
        w.account_list.remove(&child);
    }
    let current = s.current.borrow().clone();
    let config = s.config.borrow();
    for source in &config.sources {
        let is_current = current.as_deref() == Some(source.name.as_str());
        let row = account_row(&source.name, kind_label(&source.kind), is_current);
        let weak = Rc::downgrade(s);
        let name = source.name.clone();
        row.connect_clicked(move |_| {
            let Some(s) = weak.upgrade() else { return };
            s.w.account_popover.popdown();
            if s.current.borrow().as_deref() != Some(&name) {
                open_named(&s, &name);
            }
        });
        w.account_list.append(&row);
    }
    let (name, kind) = match current
        .as_deref()
        .and_then(|n| config.sources.iter().find(|c| c.name == n))
    {
        Some(c) => (c.name.clone(), kind_label(&c.kind)),
        None if s.adhoc.borrow().is_some() => ("Playlist".to_string(), "From command line"),
        None => ("Select playlist".to_string(), ""),
    };
    w.account_name.set_text(&name);
    w.account_kind.set_text(kind);
    w.account_kind.set_visible(!kind.is_empty());
    w.refresh
        .set_sensitive(current.is_some() || s.adhoc.borrow().is_some());
    w.account_menu.set_visible(true);
}

fn kind_label(kind: &SourceKind) -> &'static str {
    match kind {
        SourceKind::Xtream { .. } => "Xtream Code",
        SourceKind::M3u { .. } => "M3U playlist",
    }
}

/// Save the config to disk.
pub(crate) fn save_config(s: &S) -> Result<(), String> {
    let Some(path) = &s.config_path else {
        return Err("no config location (is $HOME set?)".into());
    };
    s.config.borrow().save(path).map_err(|e| format!("{e:#}"))
}

/// Open the source on a worker thread, then show the browser and fill it
/// as the channels download.
fn load(s: &S, resolved: Resolved) {
    let label = resolved.source.label();
    s.w.loading_label.set_text(&format!("Loading {label}…"));
    s.w.window.set_title(Some(&format!("Tuner · {label}")));
    s.w.stack.set_visible_child_name("loading");
    let generation = s.generation.get();
    let s = s.clone();
    glib::spawn_future_local(async move {
        let source = resolved.source.clone();
        let opened = gio::spawn_blocking(move || source.open())
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("loader thread panicked")));
        if s.generation.get() != generation {
            return; // the user switched to another source meanwhile
        }
        let opened = match opened {
            Ok(opened) => opened,
            Err(e) => {
                show_error(&s, &format!("Could not load channels\n\n{e:#}"));
                return;
            }
        };

        // Explicit config wins; else trust the provider's limit; else 1.
        let max_streams = resolved.max_streams.or(opened.max_connections).unwrap_or(1);
        let catalog = resolved.source.clone();
        let guard = StreamGuard::new(resolved.source.clone(), max_streams, resolved.limit_mode);
        let controller = Controller::new((s.new_player)(), guard);
        *s.controller.borrow_mut() = Some(Arc::new(Mutex::new(controller)));

        let plural = if max_streams == 1 {
            "stream"
        } else {
            "streams"
        };
        let limit = format!("limit {max_streams} {plural}");
        s.w.summary
            .set_text(&format!("Loading channels… · {limit}"));
        *s.catalog.borrow_mut() = Some(catalog);
        s.streaming.borrow_mut().insert(Section::Live);
        change_view(&s, 0, || {});

        let source = resolved.source;
        let ready = opened.clone();
        let fetched = {
            let s = s.clone();
            batched(
                move |on_batch| source.channels(&ready, on_batch),
                move |batch| {
                    if s.generation.get() != generation {
                        return Flow::Break(());
                    }
                    let start = s.live.borrow_mut().extend(batch);
                    show_added(&s, Section::Live, start);
                    Flow::Continue(())
                },
            )
            .await
        };
        if s.generation.get() != generation {
            return;
        }
        s.streaming.borrow_mut().remove(&Section::Live);
        let count = s.live.borrow().items().len();
        s.w.summary
            .set_text(&format!("{} · {limit}", opened.summary(count)));
        match fetched {
            Ok(()) => sync_loading(&s),
            Err(e) if count == 0 => show_error(&s, &format!("Could not load channels\n\n{e:#}")),
            Err(e) => {
                sync_loading(&s);
                set_status(&s, &format!("Error: {e:#} ({count} channels loaded)"));
            }
        }
    });
}

/// Run `work` on a worker thread and hand what it produces to `on_batch` on
/// the main loop, merged per [`BATCH_INTERVAL`], so a large download becomes
/// a series of small UI updates rather than one long stall. `on_batch`
/// returning `Break` abandons the work. Resolves with the work's result.
async fn batched<T: Send + 'static>(
    work: impl FnOnce(&mut dyn FnMut(Vec<T>) -> Flow) -> anyhow::Result<()> + Send + 'static,
    mut on_batch: impl FnMut(Vec<T>) -> Flow,
) -> anyhow::Result<()> {
    let (sender, batches) = mpsc::channel::<Vec<T>>();
    let done = gio::spawn_blocking(move || {
        // A failed send means the receiver is gone: nobody wants the rest.
        work(&mut |batch| match sender.send(batch) {
            Ok(()) => Flow::Continue(()),
            Err(_) => Flow::Break(()),
        })
    });
    let mut open = true;
    let mut wait = FIRST_BATCH_POLL;
    while open {
        glib::timeout_future(wait).await;
        let mut merged: Vec<T> = Vec::new();
        loop {
            match batches.try_recv() {
                Ok(batch) if merged.is_empty() => merged = batch,
                Ok(batch) => merged.extend(batch),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    open = false;
                    break;
                }
            }
        }
        wait = next_batch_wait(wait, !merged.is_empty());
        if !merged.is_empty() && on_batch(merged).is_break() {
            return Ok(()); // dropping `batches` stops the worker
        }
    }
    done.await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("loader thread panicked")))
}

/// How long [`batched`] waits before looking again: the full interval once
/// something has arrived, else twice the last wait, capped at the interval.
fn next_batch_wait(wait: Duration, delivered: bool) -> Duration {
    if delivered {
        BATCH_INTERVAL
    } else {
        (wait * 2).min(BATCH_INTERVAL)
    }
}

/// Sidebar rows for a library: "All" first, then each group with its size.
fn group_rows<T: Entry>(library: &Library<T>) -> Vec<(String, usize)> {
    let mut rows = vec![("All".to_string(), library.items().len())];
    rows.extend(
        library
            .groups()
            .iter()
            .cloned()
            .zip(library.group_sizes().iter().copied()),
    );
    rows
}

fn current_group_rows(s: &S) -> Vec<(String, usize)> {
    match s.section.get() {
        Section::Live => group_rows(&s.live.borrow()),
        Section::Movies => s
            .movies
            .borrow()
            .as_ref()
            .map(group_rows)
            .unwrap_or_default(),
        Section::Series => match s.drill.borrow().as_ref() {
            Some(drill) => group_rows(&drill.episodes),
            None => s
                .series
                .borrow()
                .as_ref()
                .map(group_rows)
                .unwrap_or_default(),
        },
    }
}

/// Rebuild the sidebar for the shown library and select row `select`, which
/// triggers the refilter.
fn fill_groups(s: &S, select: i32) {
    let mut rows = current_group_rows(s);
    if let Some(first) = rows.first_mut() {
        first.0 = nouns(s).all.to_string();
    }
    while let Some(child) = s.w.groups.first_child() {
        s.w.groups.remove(&child);
    }
    let mut counts = Vec::with_capacity(rows.len());
    for (name, count) in &rows {
        let (row, count) = group_row(name, *count);
        s.w.groups.append(&row);
        counts.push(count);
    }
    *s.group_counts.borrow_mut() = counts;
    *s.group_names.borrow_mut() = rows.into_iter().map(|(name, _)| name).collect();
    s.w.groups
        .select_row(s.w.groups.row_at_index(select).as_ref());
}

/// A sidebar row and its count label.
fn group_row(name: &str, count: usize) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(
        &gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(pango::EllipsizeMode::End)
            .build(),
    );
    let count = gtk::Label::new(Some(&count.to_string()));
    count.add_css_class("count");
    row.append(&count);
    (row, count)
}

/// Show what was just appended to `section`'s library from index `start`
/// on, if that library is on show: new sidebar groups, fresh counts, and the
/// new entries matching the current search and group at the end of the
/// list. Nothing already shown is rebuilt, so scrolling and selection stay.
fn show_added(s: &S, section: Section, start: usize) {
    if s.section.get() != section || in_episodes(s) {
        return;
    }
    let rows = current_group_rows(s);
    {
        let mut counts = s.group_counts.borrow_mut();
        let mut names = s.group_names.borrow_mut();
        for (i, (name, count)) in rows.iter().enumerate() {
            match counts.get(i) {
                Some(label) => label.set_text(&count.to_string()),
                None => {
                    let (row, label) = group_row(name, *count);
                    s.w.groups.append(&row);
                    counts.push(label);
                    names.push(name.clone());
                }
            }
        }
    }
    let (query, group) = current_filter(s);
    let items = list_objects(matching(s, start, &query, group.as_deref()));
    let was_empty = s.w.store.n_items() == 0;
    s.w.store.splice(s.w.store.n_items(), 0, &items);
    show_count(s, group.as_deref(), s.w.store.n_items() as usize);
    if was_empty && !items.is_empty() {
        s.w.results.set_visible_child_name("content");
        s.w.selection.set_selected(0);
        update_details(s);
    }
}

/// Spinner and empty-list text for whether the shown list is still
/// downloading.
fn sync_loading(s: &S) {
    let loading = !in_episodes(s) && s.streaming.borrow().contains(&s.section.get());
    s.w.list_spinner.set_visible(loading);
    s.w.list_spinner.set_spinning(loading);
    let nouns = nouns(s);
    if loading {
        s.w.no_results_title
            .set_text(&format!("Loading {}…", nouns.many));
        s.w.no_results_hint
            .set_text("They appear here as they download.");
    } else {
        s.w.no_results_title
            .set_text(&format!("No matching {}", nouns.many));
        s.w.no_results_hint.set_text("Try another search or group.");
    }
}

/// Run `f` with the entry at list `index` of whatever is shown. `None` if
/// the index is stale (the list changed under a pending callback).
fn with_entry<R>(s: &S, index: usize, f: impl FnOnce(Shown<'_>) -> R) -> Option<R> {
    match s.section.get() {
        Section::Live => s
            .live
            .borrow()
            .channels()
            .get(index)
            .map(|c| f(Shown::Channel(c))),
        Section::Movies => s
            .movies
            .borrow()
            .as_ref()
            .and_then(|l| l.channels().get(index))
            .map(|c| f(Shown::Channel(c))),
        Section::Series => match s.drill.borrow().as_ref() {
            Some(drill) => drill
                .episodes
                .channels()
                .get(index)
                .map(|c| f(Shown::Channel(c))),
            None => s
                .series
                .borrow()
                .as_ref()
                .and_then(|l| l.items().get(index))
                .map(|x| f(Shown::Series(x))),
        },
    }
}

/// Indices of the shown library's entries from `start` on that match.
fn matching(s: &S, start: usize, query: &str, group: Option<&str>) -> Vec<usize> {
    match s.section.get() {
        Section::Live => s.live.borrow().filter_from(start, query, group),
        Section::Movies => s
            .movies
            .borrow()
            .as_ref()
            .map(|l| l.filter_from(start, query, group))
            .unwrap_or_default(),
        Section::Series => match s.drill.borrow().as_ref() {
            Some(drill) => drill.episodes.filter_from(start, query, group),
            None => s
                .series
                .borrow()
                .as_ref()
                .map(|l| l.filter_from(start, query, group))
                .unwrap_or_default(),
        },
    }
}

fn in_episodes(s: &S) -> bool {
    s.section.get() == Section::Series && s.drill.borrow().is_some()
}

fn nouns(s: &S) -> Nouns {
    match (s.section.get(), in_episodes(s)) {
        (_, true) => Nouns {
            all: "All episodes",
            one: "episode",
            many: "episodes",
            heading: "Seasons",
        },
        (Section::Live, _) => Nouns {
            all: "All channels",
            one: "channel",
            many: "channels",
            heading: "Your channels",
        },
        (Section::Movies, _) => Nouns {
            all: "All movies",
            one: "movie",
            many: "movies",
            heading: "Movie categories",
        },
        (Section::Series, _) => Nouns {
            all: "All series",
            one: "series",
            many: "series",
            heading: "Series categories",
        },
    }
}

/// Bring the header, sidebar heading and search hint in line with what is
/// shown.
fn sync_chrome(s: &S) {
    let section = s.section.get();
    let has_vod = s.catalog.borrow().as_ref().is_some_and(|c| c.has_vod());
    s.syncing.set(true);
    for (sec, button) in &s.w.sections {
        button.set_active(*sec == section);
        let enabled = *sec == Section::Live || has_vod;
        button.set_sensitive(enabled);
        button.set_tooltip_text(match (*sec, enabled) {
            (Section::Live, _) => Some("Live TV (Ctrl+1)"),
            (Section::Movies, true) => Some("Movies (Ctrl+2)"),
            (Section::Series, true) => Some("Series (Ctrl+3)"),
            _ => Some("Needs an Xtream account"),
        });
    }
    s.syncing.set(false);
    s.w.back.set_visible(in_episodes(s));
    s.w.groups_heading.set_text(nouns(s).heading);
    s.w.search
        .set_placeholder_text(Some(match (section, in_episodes(s)) {
            (_, true) => "Search episodes",
            (Section::Live, _) => "Search channels",
            (Section::Movies, _) => "Search movies",
            (Section::Series, _) => "Search series",
        }));
}

/// Swap what the list shows: `apply` changes the state, then the sidebar
/// (selecting group row `select`), list and header are rebuilt with a single
/// filter pass. The list is emptied first so no row is bound to an index of
/// the old library.
fn change_view(s: &S, select: i32, apply: impl FnOnce()) {
    s.rebuilding.set(true);
    s.w.store.remove_all();
    apply();
    s.w.search.set_text("");
    fill_groups(s, select);
    s.rebuilding.set(false);
    sync_chrome(s);
    s.w.stack.set_visible_child_name("channels");
    refilter(s);
    focus_channels(s);
    fade_in(s);
}

/// Fade the list and sidebar in after a change, so switching sections reads
/// as a transition rather than a jump. Skipped when the desktop turns
/// animations off.
fn fade_in(s: &S) {
    const DURATION_US: f64 = 180_000.0;
    let generation = s.fade.get().wrapping_add(1);
    s.fade.set(generation);
    let animate = gtk::Settings::default().is_some_and(|d| d.is_gtk_enable_animations());
    let opacity = if animate { 0.0 } else { 1.0 };
    s.w.results.set_opacity(opacity);
    s.w.groups.set_opacity(opacity);
    if !animate {
        return;
    }
    let weak = Rc::downgrade(s);
    let start: Cell<Option<i64>> = Cell::new(None);
    s.w.results.add_tick_callback(move |_, clock| {
        let Some(s) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if s.fade.get() != generation {
            return glib::ControlFlow::Break;
        }
        let now = clock.frame_time();
        let begin = start.get().unwrap_or(now);
        start.set(Some(begin));
        let progress = ((now - begin) as f64 / DURATION_US).clamp(0.0, 1.0);
        // Ease out: quick start, gentle landing.
        let eased = 1.0 - (1.0 - progress).powi(3);
        s.w.results.set_opacity(eased);
        s.w.groups.set_opacity(eased);
        if progress >= 1.0 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn switch_section(s: &S, target: Section) {
    if target == s.section.get() {
        return;
    }
    let loaded = match target {
        Section::Live => true,
        Section::Movies => s.movies.borrow().is_some(),
        Section::Series => s.series.borrow().is_some(),
    };
    if loaded {
        change_view(s, 0, || s.section.set(target));
        return;
    }
    let source = s.catalog.borrow().clone().filter(|c| c.has_vod());
    let Some(source) = source else {
        sync_chrome(s);
        set_status(s, "Movies and series need an Xtream account");
        return;
    };
    let what = if target == Section::Movies {
        "movies"
    } else {
        "series"
    };
    // Show the (empty) section at once; entries appear as they download.
    s.streaming.borrow_mut().insert(target);
    change_view(s, 0, || {
        match target {
            Section::Movies => *s.movies.borrow_mut() = Some(Library::default()),
            _ => *s.series.borrow_mut() = Some(Library::default()),
        }
        s.section.set(target);
    });
    set_status(s, &format!("Loading {what}…"));
    let generation = s.generation.get();
    let s = s.clone();
    glib::spawn_future_local(async move {
        let current = {
            let s = s.clone();
            move || s.generation.get() == generation
        };
        let fetched = match target {
            Section::Movies => {
                let s = s.clone();
                batched(
                    move |on_batch| source.movies_batched(on_batch),
                    move |batch| {
                        if !current() {
                            return Flow::Break(());
                        }
                        let start = s.movies.borrow_mut().get_or_insert_default().extend(batch);
                        show_added(&s, target, start);
                        Flow::Continue(())
                    },
                )
                .await
            }
            _ => {
                let s = s.clone();
                batched(
                    move |on_batch| source.series_batched(on_batch),
                    move |batch| {
                        if !current() {
                            return Flow::Break(());
                        }
                        let start = s.series.borrow_mut().get_or_insert_default().extend(batch);
                        show_added(&s, target, start);
                        Flow::Continue(())
                    },
                )
                .await
            }
        };
        if s.generation.get() != generation {
            return; // the user switched account meanwhile
        }
        s.streaming.borrow_mut().remove(&target);
        let count = match target {
            Section::Movies => s.movies.borrow().as_ref().map_or(0, |l| l.items().len()),
            _ => s.series.borrow().as_ref().map_or(0, |l| l.items().len()),
        };
        sync_loading(&s);
        match fetched {
            Ok(()) => set_status(&s, &format!("Loaded {count} {what}")),
            Err(e) => {
                if count == 0 {
                    // Nothing arrived: forget the attempt so the next visit
                    // retries, and go back to live TV.
                    match target {
                        Section::Movies => *s.movies.borrow_mut() = None,
                        _ => *s.series.borrow_mut() = None,
                    }
                    if s.section.get() == target {
                        change_view(&s, 0, || s.section.set(Section::Live));
                    }
                    set_status(&s, &format!("Error: {e:#}"));
                } else {
                    set_status(&s, &format!("Error: {e:#} ({count} {what} loaded)"));
                }
                inform(&s, &format!("Could not load {what}"), &format!("{e:#}")).await;
            }
        }
    });
}

/// Fetch a series' episodes and list them (seasons become the sidebar).
fn open_series(s: &S, series: Series) {
    let source = s.catalog.borrow().clone();
    let Some(source) = source else {
        return;
    };
    if s.vod_busy.replace(true) {
        return;
    }
    s.w.loading_label
        .set_text(&format!("Loading {}…", series.name));
    s.w.stack.set_visible_child_name("loading");
    set_status(s, &format!("Loading {}…", series.name));
    let back_group = s.w.groups.selected_row().map_or(0, |r| r.index());
    let generation = s.generation.get();
    let s = s.clone();
    glib::spawn_future_local(async move {
        let id = series.id.clone();
        let fetched = gio::spawn_blocking(move || source.episodes(&id).map(Library::new))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("loader thread panicked")));
        if s.generation.get() != generation {
            return;
        }
        s.vod_busy.set(false);
        match fetched {
            Ok(episodes) if episodes.items().is_empty() => {
                s.w.stack.set_visible_child_name("channels");
                set_status(&s, &format!("No episodes found for {}", series.name));
            }
            Ok(episodes) => {
                set_status(
                    &s,
                    &format!("{}: {} episodes", series.name, episodes.items().len()),
                );
                change_view(&s, 0, || {
                    *s.drill.borrow_mut() = Some(Drill {
                        episodes,
                        back_group,
                    });
                });
            }
            Err(e) => {
                s.w.stack.set_visible_child_name("channels");
                set_status(&s, &format!("Error: {e:#}"));
                inform(&s, "Could not load episodes", &format!("{e:#}")).await;
            }
        }
    });
}

/// Leave a series' episodes for the series list, on the group left from.
fn close_series(s: &S) {
    let Some(back_group) = s.drill.borrow().as_ref().map(|d| d.back_group) else {
        return;
    };
    change_view(s, back_group, || *s.drill.borrow_mut() = None);
    set_status(s, "");
}

/// The search text and the selected sidebar group (`None` for "All").
fn current_filter(s: &S) -> (glib::GString, Option<String>) {
    let group =
        s.w.groups
            .selected_row()
            .map(|r| r.index())
            .filter(|&i| i > 0)
            .and_then(|i| s.group_names.borrow().get(i as usize).cloned());
    (s.w.search.text(), group)
}

fn list_objects(indices: Vec<usize>) -> Vec<glib::BoxedAnyObject> {
    indices.into_iter().map(glib::BoxedAnyObject::new).collect()
}

fn refilter(s: &S) {
    let (query, group) = current_filter(s);
    let items = list_objects(matching(s, 0, &query, group.as_deref()));
    s.w.store.splice(0, s.w.store.n_items(), &items);
    show_count(s, group.as_deref(), items.len());
    s.w.results
        .set_visible_child_name(if items.is_empty() { "empty" } else { "content" });
    if !items.is_empty() {
        s.w.selection.set_selected(0);
    }
    update_details(s);
    sync_loading(s);
}

/// List heading: the group (or "All …") and how many entries it shows.
fn show_count(s: &S, group: Option<&str>, count: usize) {
    let nouns = nouns(s);
    s.w.channels_title.set_text(group.unwrap_or(nouns.all));
    s.w.channels_count.set_text(&format!(
        "{count} {}",
        if count == 1 { nouns.one } else { nouns.many }
    ));
}

fn item_index(obj: Option<glib::Object>) -> Option<usize> {
    obj.and_downcast::<glib::BoxedAnyObject>()
        .map(|o| *o.borrow::<usize>())
}

fn selected_index(s: &S) -> Option<usize> {
    item_index(s.w.selection.selected_item())
}

/// "2002 · ★ 9.3" for a series row.
fn series_info(series: &Series) -> String {
    let mut parts = Vec::new();
    parts.extend(series.year.clone());
    parts.extend(series.rating.as_ref().map(|r| format!("★ {r}")));
    parts.join(" · ")
}

/// Fill the details grid from `fields`; unused rows are hidden.
fn show_fields(d: &Details, fields: &[(&str, String)]) {
    for (i, (name, value)) in d.names.iter().zip(&d.values).enumerate() {
        match fields.get(i) {
            Some((label, text)) => {
                name.set_text(label);
                value.set_text(text);
                // A URL has no word breaks: wrap at any character, in a small
                // monospace face, so lines fill the column evenly.
                let url = *label == "URL";
                value.set_wrap_mode(if url {
                    pango::WrapMode::Char
                } else {
                    pango::WrapMode::WordChar
                });
                value.set_max_width_chars(if url { 28 } else { 18 });
                name.set_valign(gtk::Align::Start);
                if url {
                    value.add_css_class("url-value");
                } else {
                    value.remove_css_class("url-value");
                }
            }
            None => {
                name.set_text("");
                value.set_text("");
            }
        }
        name.set_visible(i < fields.len());
        value.set_visible(i < fields.len());
    }
}

/// What the details panel shows for the selected entry.
struct Detail {
    title: String,
    subtitle: String,
    fields: Vec<(&'static str, String)>,
    logo: Option<String>,
    /// Whether the entry has a stream URL to reveal.
    has_url: bool,
    /// Button label: "Play …" or "Show episodes".
    action: &'static str,
}

fn update_details(s: &S) {
    let d = &s.w.details;
    let live = s.section.get() == Section::Live;
    let detail = selected_index(s).and_then(|index| {
        with_entry(s, index, |entry| match entry {
            Shown::Channel(c) => {
                let dash = || "–".to_string();
                let mut subtitle = vec![c.group.as_deref().unwrap_or("Ungrouped").to_string()];
                let mut fields = vec![("Group", c.group.clone().unwrap_or_else(dash))];
                if live {
                    if let Some(number) = c.number {
                        subtitle.push(format!("Channel {number}"));
                    }
                    fields.push(("Number", c.number.map_or_else(dash, |n| n.to_string())));
                    fields.push((
                        "Catch-up",
                        c.catchup_days
                            .map_or_else(|| "none".into(), |d| format!("{d} days")),
                    ));
                    fields.push(("EPG id", c.tvg_id.clone().unwrap_or_else(dash)));
                }
                fields.push(("Format", c.stream_format().to_string()));
                // Host only: stream URLs usually embed the account password.
                fields.push(("Server", tuner_app::source::host(&c.url).to_string()));
                if d.reveal_url.is_active() {
                    fields.push(("URL", c.url.clone()));
                }
                Detail {
                    title: c.name.clone(),
                    subtitle: subtitle.join(" · "),
                    fields,
                    logo: c.logo.clone(),
                    has_url: true,
                    action: match (live, in_episodes(s)) {
                        (true, _) => "Play channel",
                        (_, true) => "Play episode",
                        _ => "Play movie",
                    },
                }
            }
            Shown::Series(x) => {
                let dash = || "–".to_string();
                let mut fields = vec![
                    ("Category", x.group.clone().unwrap_or_else(dash)),
                    ("Year", x.year.clone().unwrap_or_else(dash)),
                    ("Rating", x.rating.clone().unwrap_or_else(dash)),
                ];
                if let Some(plot) = &x.plot {
                    fields.push(("Plot", plot.clone()));
                }
                Detail {
                    title: x.name.clone(),
                    subtitle: series_info(x),
                    fields,
                    logo: x.cover.clone(),
                    has_url: false,
                    action: "Show episodes",
                }
            }
        })
    });
    let Some(detail) = detail else {
        d.title.set_text("Nothing selected");
        d.subtitle.set_text("Choose an entry to see its details.");
        show_fields(d, &[]);
        d.reveal_url.set_visible(false);
        d.play_label.set_text("Play channel");
        d.play.set_sensitive(false);
        s.logos.show(None, "", &d.logo);
        return;
    };
    d.title.set_text(&detail.title);
    d.subtitle.set_text(&detail.subtitle);
    show_fields(d, &detail.fields);
    d.reveal_url.set_visible(detail.has_url);
    d.play_label.set_text(detail.action);
    d.play
        .update_property(&[gtk::accessible::Property::Label(detail.action)]);
    d.play.set_sensitive(true);
    s.logos.show(detail.logo.as_deref(), &detail.title, &d.logo);
}

fn set_status(s: &S, text: &str) {
    s.w.status.set_text(text);
}

fn focus_channels(s: &S) {
    if s.w.cards_toggle.is_active() {
        s.w.cards.grab_focus();
    } else {
        s.w.list.grab_focus();
    }
}

/// Bound both sidebars while leaving room for the channel browser. Inputs
/// are preferences; a temporarily narrow window need not discard them.
fn pane_widths(width: i32, groups: i32, details: i32, show_details: bool) -> (i32, i32) {
    let reserved = if show_details {
        DETAILS_MIN + HANDLE_WIDTH
    } else {
        0
    };
    let groups_max = (width - BROWSER_MIN - reserved - HANDLE_WIDTH).clamp(GROUPS_MIN, GROUPS_MAX);
    let groups = groups.clamp(GROUPS_MIN, groups_max);
    let details = if show_details {
        let maximum =
            (width - groups - BROWSER_MIN - 2 * HANDLE_WIDTH).clamp(DETAILS_MIN, DETAILS_MAX);
        details.clamp(DETAILS_MIN, maximum)
    } else {
        0
    };
    (groups, details)
}

fn update_details_visibility(s: &S) {
    let width = s.w.body_size.widget().map_or(0, |body| body.width());
    let browsing = s.w.stack.visible_child_name().as_deref() == Some("channels");
    let show = width >= DETAILS_BREAKPOINT && browsing && s.w.details_toggle.is_active();
    let (groups, details) = pane_widths(width, s.groups_width.get(), s.details_width.get(), show);
    s.w.details.revealer.set_visible(show);
    s.w.details.revealer.set_reveal_child(show);
    s.w.sidebar.set_width_request(groups);
    s.w.details_handle.set_visible(show);
    if let Some(pane) = s.w.details.revealer.child() {
        pane.set_width_request(details.max(DETAILS_MIN));
    }
}

/// The marker text for the row at list `index`: only the playing channel,
/// movie or episode is marked.
fn marker_text(s: &S, index: usize) -> &'static str {
    let playing = with_entry(
        s,
        index,
        |entry| matches!(entry, Shown::Channel(c) if s.is_playing(c)),
    );
    if playing != Some(true) {
        ""
    } else if s.paused.get() {
        "Ⅱ Paused"
    } else {
        "▶ Playing"
    }
}

fn update_playing_markers(s: &S) {
    let markers: Vec<_> = s
        .markers
        .borrow()
        .iter()
        .filter_map(|(&(_, index), label)| Some((index, label.upgrade()?)))
        .collect();
    for (index, label) in markers {
        label.set_text(marker_text(s, index));
    }
}

fn set_playing(s: &S, channel: Option<Channel>) {
    *s.now.borrow_mut() = channel.clone();
    s.paused.set(false);
    // The new channel may be in another section, so re-mark bound rows.
    update_playing_markers(s);

    if let Some(video) = &s.w.video {
        let detached = video.detached.load(Ordering::Relaxed);
        let show = channel.is_some() && !detached;
        // The video gets the details panel's room while it plays (Ctrl+I
        // still opens it); the panel comes back when the video closes.
        if show && !video.widget.is_visible() {
            s.details_before_video
                .set(Some(s.w.details_toggle.is_active()));
            s.w.details_toggle.set_active(false);
        } else if !show && let Some(was) = s.details_before_video.take() {
            s.w.details_toggle.set_active(was);
        }
        video.widget.set_visible(show);
    }
    let bar = &s.w.player;
    match channel {
        Some(c) => {
            bar.title.set_text(&c.name);
            let mut sub: Vec<String> = Vec::new();
            if let Some(n) = c.number {
                sub.push(format!("#{n}"));
            }
            if let Some(g) = &c.group {
                sub.push(g.clone());
            }
            sub.push(c.stream_format().to_string());
            bar.subtitle.set_text(&sub.join(" · "));
            s.logos.show(c.logo.as_deref(), &c.name, &bar.logo);
            set_paused(s, false);
            bar.revealer.set_reveal_child(true);
        }
        None => bar.revealer.set_reveal_child(false),
    }
}

/// Run `f` on the controller in a worker thread (mpv start and the provider
/// check block). `None` if the worker panicked.
async fn with_controller<T: Send + 'static>(
    ctl: &Ctl,
    f: impl FnOnce(&mut Controller<AnyPlayer, StreamGuard>) -> T + Send + 'static,
) -> Option<T> {
    let ctl = ctl.clone();
    gio::spawn_blocking(move || f(&mut ctl.lock().unwrap_or_else(|e| e.into_inner())))
        .await
        .ok()
}

/// Enter / double-click / the details button: play a channel, movie or
/// episode; open a series to its episodes.
fn activate(s: &S, index: usize) {
    enum Act {
        Play(Channel),
        Open(Series),
    }
    let act = with_entry(s, index, |entry| match entry {
        Shown::Channel(c) => Act::Play(c.clone()),
        Shown::Series(x) => Act::Open(x.clone()),
    });
    match act {
        Some(Act::Play(channel)) => play(s, channel),
        Some(Act::Open(series)) => open_series(s, series),
        None => {}
    }
}

fn play(s: &S, channel: Channel) {
    let Some(ctl) = s.controller() else {
        return;
    };
    if s.busy.replace(true) {
        return;
    }
    let url = channel.url.clone();
    set_status(
        s,
        if s.now.borrow().is_some() {
            "Switching…"
        } else {
            "Checking stream limit…"
        },
    );
    let generation = s.generation.get();
    let s = s.clone();
    glib::spawn_future_local(async move {
        let u = url.clone();
        let mut outcome = with_controller(&ctl, move |c| c.request(SLOT, &u))
            .await
            .unwrap_or_else(|| Request::Failed("player thread panicked".into()));

        if let Request::NeedsConfirm(message) = &outcome {
            if confirm(&s, message).await {
                outcome = with_controller(&ctl, move |c| c.confirm(SLOT, &url))
                    .await
                    .unwrap_or_else(|| Request::Failed("player thread panicked".into()));
            } else {
                with_controller(&ctl, |c| c.cancel()).await;
                set_status(&s, "Cancelled: nothing was opened");
                s.busy.set(false);
                return;
            }
        }

        if s.generation.get() != generation {
            // Switched account meanwhile; that controller is shut down.
            return;
        }
        match outcome {
            Request::Started => {
                let progress = with_controller(&ctl, |c| c.progress()).await.flatten();
                s.stage.set(progress.as_ref().map(|p| p.stage));
                set_status(&s, &status_line(&channel.name, progress.as_ref()));
                set_playing(&s, Some(channel));
            }
            Request::Blocked(message) => {
                set_status(&s, "Blocked: stream limit reached");
                inform(&s, "Stream limit reached", &message).await;
            }
            Request::Failed(e) => set_status(&s, &format!("Error: {e}")),
            Request::NeedsConfirm(_) => {}
        }
        s.busy.set(false);
    });
}

fn stop(s: &S) {
    let Some(ctl) = s.controller() else {
        return;
    };
    let s = s.clone();
    glib::spawn_future_local(async move {
        let stopped = with_controller(&ctl, |c| c.stop()).await;
        set_playing(&s, None);
        set_status(
            &s,
            if stopped.is_some() {
                "Stopped"
            } else {
                "Error: player thread panicked"
            },
        );
    });
}

/// Move the playing stream from the embedded video to mpv's own window. It
/// replays through the controller like a channel switch, so the stream slot
/// is kept and only one stream is ever open.
fn detach(s: &S) {
    let (Some(video), Some(channel)) = (&s.w.video, s.now.borrow().clone()) else {
        return;
    };
    // `play` ignores a click while another play runs; the flag would then
    // send some later stream to mpv's window instead.
    if s.busy.get() {
        return;
    }
    video.detach_next.store(true, Ordering::Relaxed);
    play(s, channel);
}

fn toggle_pause(s: &S) {
    let Some(ctl) = s.controller() else {
        return;
    };
    let s = s.clone();
    glib::spawn_future_local(async move {
        let result = with_controller(&ctl, |c| c.toggle_pause().map(|()| c.is_paused())).await;
        match result {
            Some(Ok(Some(paused))) => {
                set_paused(&s, paused);
                set_status(&s, if paused { "Paused" } else { "Playing" });
            }
            Some(Ok(None)) => {}
            Some(Err(e)) => set_status(&s, &format!("Error: {e}")),
            None => set_status(&s, "Error: player thread panicked"),
        }
    });
}

async fn confirm(s: &S, message: &str) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message("Stream limit")
        .detail(message)
        .buttons(["Cancel", "Play anyway"])
        .cancel_button(0)
        .default_button(0)
        .build();
    matches!(dialog.choose_future(Some(&s.w.window)).await, Ok(1))
}

async fn inform(s: &S, title: &str, message: &str) {
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(title)
        .detail(message)
        .buttons(["OK"])
        .build();
    let _ = dialog.choose_future(Some(&s.w.window)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_are_polled_soon_until_the_first_arrives() {
        let mut wait = FIRST_BATCH_POLL;
        let mut total = Duration::ZERO;
        for _ in 0..4 {
            total += wait;
            wait = next_batch_wait(wait, false);
        }
        assert!(total < Duration::from_millis(80), "a fast load shows early");
        for _ in 0..10 {
            wait = next_batch_wait(wait, false);
        }
        assert_eq!(wait, BATCH_INTERVAL, "a slow download backs off fully");
        assert_eq!(next_batch_wait(FIRST_BATCH_POLL, true), BATCH_INTERVAL);
    }

    #[test]
    fn theme_rows_round_trip() {
        assert_eq!(choice_for_row(0), theme::Choice::Custom);
        assert_eq!(theme_row(theme::Choice::Custom), 0);
        for row in 1..=themes::ALL.len() {
            assert_eq!(theme_row(choice_for_row(row)), row);
        }
        // Out of range falls back to the theme file rather than panicking.
        assert_eq!(choice_for_row(themes::ALL.len() + 1), theme::Choice::Custom);
    }

    #[test]
    fn sidebar_limits_always_leave_a_usable_browser() {
        for width in [520, 600, 820, 1000, 1180, 1600, 2400] {
            for groups in [-100, 180, 200, 360, 10_000] {
                for details in [-100, 260, 320, 440, 10_000] {
                    for requested in [false, true] {
                        let show = requested && width >= DETAILS_BREAKPOINT;
                        let (left, right) = pane_widths(width, groups, details, show);
                        assert!((GROUPS_MIN..=GROUPS_MAX).contains(&left));
                        if show {
                            assert!((DETAILS_MIN..=DETAILS_MAX).contains(&right));
                        } else {
                            assert_eq!(right, 0);
                        }
                        let handles = if show { 2 } else { 1 } * HANDLE_WIDTH;
                        assert!(width - left - right - handles >= BROWSER_MIN);
                    }
                }
            }
        }
    }

    #[test]
    fn sidebar_preferences_are_only_constrained_when_space_requires_it() {
        assert_eq!(pane_widths(1180, 275, 390, true), (275, 390));
        assert_eq!(pane_widths(1000, 360, 440, true), (360, 328));
        assert_eq!(pane_widths(1180, 275, 390, false), (275, 0));
    }
}
