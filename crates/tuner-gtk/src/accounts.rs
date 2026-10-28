//! The Accounts window: add, edit, test, remove and reorder the sources in
//! `config.toml` without editing the file. Changes are validated and saved
//! through `tuner_app::Config`, so the TUI sees them too. See `docs/gui.md`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib, pango};
use tuner_app::LimitMode;
use tuner_app::config::{SourceConfig, SourceKind};

use crate::window::{self, S};

const TYPES: [&str; 2] = ["Xtream Codes", "M3U playlist"];
const LIMIT_MODES: [&str; 2] = ["Ask before playing", "Never play"];

struct Form {
    window: gtk::Window,
    list: gtk::ListBox,
    remove: gtk::Button,
    make_default: gtk::Button,
    title: gtk::Label,
    name: gtk::Entry,
    kind: gtk::DropDown,
    xtream_rows: Vec<gtk::Widget>,
    m3u_rows: Vec<gtk::Widget>,
    server: gtk::Entry,
    username: gtk::Entry,
    password: gtk::PasswordEntry,
    password_command: gtk::Entry,
    location: gtk::Entry,
    max_streams: gtk::SpinButton,
    limit_mode: gtk::DropDown,
    test: gtk::Button,
    save: gtk::Button,
    status: gtk::Label,
    /// Name of the account being edited; `None` while adding a new one.
    editing: RefCell<Option<String>>,
    /// Set while rows are refilled, so selection changes don't reload the form.
    syncing: Cell<bool>,
}

type F = Rc<Form>;

/// Show the Accounts window. With `new`, start on an empty "add" form.
pub fn open(s: &S, new: bool) {
    let f = build(s);
    connect(s, &f);
    refresh_list(s, &f);
    let current = s.current.borrow().clone();
    match current.filter(|_| !new) {
        Some(name) => select(&f, &name),
        None if !new && !s.config.borrow().sources.is_empty() => {
            let first = s.config.borrow().sources[0].name.clone();
            select(&f, &first);
        }
        _ => start_new(&f),
    }
    // The window owns the form state. All handlers hold `Weak` references,
    // so without this the state would be freed as soon as `open` returns and
    // every button would silently do nothing.
    let owner = RefCell::new(Some(f.clone()));
    f.window.connect_close_request(move |_| {
        owner.borrow_mut().take();
        glib::Propagation::Proceed
    });
    f.window.present();
}

fn build(s: &S) -> F {
    let window = gtk::Window::builder()
        .title("Accounts")
        .transient_for(&s.w.window)
        .modal(true)
        .default_width(820)
        .default_height(520)
        .build();

    // Left: account list and its actions.
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let add = icon_button("list-add-symbolic", "Add account (Ctrl+N)");
    let remove = icon_button("list-remove-symbolic", "Remove account");
    let make_default = gtk::Button::builder()
        .label("Make _default")
        .use_underline(true)
        .tooltip_text("Open this account when tuner starts")
        .build();
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.append(&add);
    actions.append(&remove);
    actions.append(&make_default);
    let left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .width_request(250)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    left.append(&scroll);
    left.append(&actions);
    let left_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left_frame.add_css_class("sidebar");
    left_frame.append(&left);

    // Right: the form.
    let title = gtk::Label::builder().xalign(0.0).build();
    title.add_css_class("details-title");
    let grid = gtk::Grid::builder()
        .row_spacing(10)
        .column_spacing(14)
        .build();
    let mut row = 0;
    let mut add_row = |label: &str, widget: &gtk::Widget| -> Vec<gtk::Widget> {
        let l = gtk::Label::builder().label(label).xalign(0.0).build();
        l.add_css_class("field-label");
        widget.set_hexpand(true);
        // Screen readers (and UI tests) find fields by their label.
        widget.update_relation(&[gtk::accessible::Relation::LabelledBy(&[l.upcast_ref()])]);
        grid.attach(&l, 0, row, 1, 1);
        grid.attach(widget, 1, row, 1, 1);
        row += 1;
        vec![l.upcast(), widget.clone()]
    };

    let name = gtk::Entry::builder()
        .placeholder_text("e.g. Home IPTV")
        .build();
    add_row("Name", name.upcast_ref());
    let kind = gtk::DropDown::from_strings(&TYPES);
    add_row("Type", kind.upcast_ref());

    let server = gtk::Entry::builder()
        .placeholder_text("http://line.example.com:8080")
        .input_purpose(gtk::InputPurpose::Url)
        .build();
    let username = gtk::Entry::new();
    let password = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let password_command = gtk::Entry::builder()
        .placeholder_text("optional, instead of a password: pass show iptv/home")
        .build();
    let mut xtream_rows = add_row("Server", server.upcast_ref());
    xtream_rows.extend(add_row("Username", username.upcast_ref()));
    xtream_rows.extend(add_row("Password", password.upcast_ref()));
    xtream_rows.extend(add_row("Password command", password_command.upcast_ref()));

    let location = gtk::Entry::builder()
        .placeholder_text("https://…/playlist.m3u  or  /path/to/playlist.m3u")
        .build();
    let m3u_rows = add_row("Playlist", location.upcast_ref());

    let max_streams = gtk::SpinButton::with_range(0.0, 20.0, 1.0);
    max_streams.set_tooltip_text(Some(
        "Streams this account may play at once. 0 = use the provider's limit (or 1).",
    ));
    max_streams.set_hexpand(false);
    add_row("Max streams", max_streams.upcast_ref());
    let limit_mode = gtk::DropDown::from_strings(&LIMIT_MODES);
    limit_mode.set_tooltip_text(Some(
        "What to do when the provider reports the account as full.",
    ));
    add_row("When account is full", limit_mode.upcast_ref());

    let note = gtk::Label::builder()
        .label(
            "Passwords are saved in ~/.config/tuner/config.toml (readable only by you). \
             Use a password command to keep them in a password manager instead.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    note.add_css_class("dim-label");

    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .build();
    let test = gtk::Button::with_mnemonic("_Test login");
    test.set_tooltip_text(Some("Ctrl+T"));
    let save = gtk::Button::with_mnemonic("_Save");
    save.set_tooltip_text(Some("Enter or Ctrl+S"));
    save.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&test);
    buttons.append(&save);

    let right = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .hexpand(true)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(18)
        .margin_end(18)
        .build();
    right.append(&title);
    right.append(&grid);
    right.append(&note);
    right.append(&status);
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    right.append(&spacer);
    right.append(&buttons);

    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.append(&left_frame);
    root.append(&right);
    window.set_child(Some(&root));

    let f = Rc::new(Form {
        window,
        list,
        remove,
        make_default,
        title,
        name,
        kind,
        xtream_rows,
        m3u_rows,
        server,
        username,
        password,
        password_command,
        location,
        max_streams,
        limit_mode,
        test,
        save,
        status,
        editing: RefCell::new(None),
        syncing: Cell::new(false),
    });

    let weak = Rc::downgrade(&f);
    add.connect_clicked(move |_| {
        if let Some(f) = weak.upgrade() {
            f.list.unselect_all();
            start_new(&f);
        }
    });
    let new = gtk::ShortcutController::new();
    new.set_propagation_phase(gtk::PropagationPhase::Capture);
    for (trigger, button) in [
        ("<Control>n", add.clone()),
        ("<Control>t", f.test.clone()),
        ("<Control>s", f.save.clone()),
    ] {
        new.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(trigger),
            Some(gtk::CallbackAction::new(move |_, _| {
                button.emit_clicked();
                glib::Propagation::Stop
            })),
        ));
    }
    f.window.add_controller(new);
    // Enter in any field saves.
    f.window.set_default_widget(Some(&f.save));
    for entry in [
        &f.name,
        &f.server,
        &f.username,
        &f.password_command,
        &f.location,
    ] {
        entry.set_activates_default(true);
    }
    f.password.set_activates_default(true);
    f
}

fn connect(s: &S, f: &F) {
    let weak = Rc::downgrade(f);
    f.kind.connect_selected_notify(move |_| {
        if let Some(f) = weak.upgrade() {
            show_kind_rows(&f);
        }
    });

    let (sw, fw) = (Rc::downgrade(s), Rc::downgrade(f));
    f.list.connect_row_selected(move |_, row| {
        let (Some(s), Some(f)) = (sw.upgrade(), fw.upgrade()) else {
            return;
        };
        if f.syncing.get() {
            return;
        }
        let name = row.and_then(|r| {
            s.config
                .borrow()
                .sources
                .get(r.index() as usize)
                .map(|c| c.name.clone())
        });
        if let Some(name) = name {
            fill(&s, &f, &name);
        }
    });

    let (sw, fw) = (Rc::downgrade(s), Rc::downgrade(f));
    f.save.connect_clicked(move |_| {
        if let (Some(s), Some(f)) = (sw.upgrade(), fw.upgrade()) {
            save(&s, &f);
        }
    });

    let fw = Rc::downgrade(f);
    f.test.connect_clicked(move |_| {
        if let Some(f) = fw.upgrade() {
            test_login(&f);
        }
    });

    let (sw, fw) = (Rc::downgrade(s), Rc::downgrade(f));
    f.remove.connect_clicked(move |_| {
        if let (Some(s), Some(f)) = (sw.upgrade(), fw.upgrade()) {
            glib::spawn_future_local(async move { remove(&s, &f).await });
        }
    });

    let (sw, fw) = (Rc::downgrade(s), Rc::downgrade(f));
    f.make_default.connect_clicked(move |_| {
        let (Some(s), Some(f)) = (sw.upgrade(), fw.upgrade()) else {
            return;
        };
        let Some(name) = f.editing.borrow().clone() else {
            return;
        };
        s.config.borrow_mut().make_default(&name);
        match window::save_config(&s) {
            Ok(()) => set_status(&f, &format!("{name} now opens by default."), false),
            Err(e) => set_status(&f, &format!("Could not save: {e}"), true),
        }
        refresh_list(&s, &f);
        select(&f, &name);
        window::sync_sources(&s);
    });

    let close = gtk::ShortcutController::new();
    let fw = Rc::downgrade(f);
    close.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("Escape"),
        Some(gtk::CallbackAction::new(move |_, _| {
            if let Some(f) = fw.upgrade() {
                f.window.close();
            }
            glib::Propagation::Stop
        })),
    ));
    f.window.add_controller(close);
}

fn refresh_list(s: &S, f: &F) {
    f.syncing.set(true);
    while let Some(child) = f.list.first_child() {
        f.list.remove(&child);
    }
    let config = s.config.borrow();
    for (i, source) in config.sources.iter().enumerate() {
        let (kind, detail) = match &source.kind {
            SourceKind::Xtream {
                server, username, ..
            } => (
                "Xtream",
                format!("{username} @ {}", tuner_app::source::host(server)),
            ),
            SourceKind::M3u { location } => ("M3U", short_location(location)),
        };
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let name = gtk::Label::builder()
            .label(if i == 0 {
                format!("{}  · default", source.name)
            } else {
                source.name.clone()
            })
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        let sub = gtk::Label::builder()
            .label(format!("{kind} · {detail}"))
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        sub.add_css_class("dim-label");
        row.append(&name);
        row.append(&sub);
        f.list.append(&row);
    }
    f.syncing.set(false);
}

/// Path, or URL reduced to its host so `get.php?…&password=` never shows.
fn short_location(location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        tuner_app::source::host(location).to_string()
    } else {
        location.to_string()
    }
}

fn select(f: &F, name: &str) {
    let index = (0..)
        .map_while(|i| f.list.row_at_index(i))
        .find(|row| row_name(row).as_deref() == Some(name));
    match index {
        Some(row) => f.list.select_row(Some(&row)),
        None => start_new(f),
    }
}

fn row_name(row: &gtk::ListBoxRow) -> Option<String> {
    let text = row
        .child()?
        .first_child()
        .and_downcast::<gtk::Label>()?
        .text();
    Some(text.trim_end_matches("  · default").to_string())
}

fn start_new(f: &F) {
    *f.editing.borrow_mut() = None;
    f.title.set_text("New account");
    for entry in [
        &f.name,
        &f.server,
        &f.username,
        &f.password_command,
        &f.location,
    ] {
        entry.set_text("");
    }
    f.password.set_text("");
    f.kind.set_selected(0);
    f.max_streams.set_value(0.0);
    f.limit_mode.set_selected(0);
    f.remove.set_sensitive(false);
    f.make_default.set_sensitive(false);
    set_status(f, "", false);
    show_kind_rows(f);
    f.name.grab_focus();
}

fn fill(s: &S, f: &F, name: &str) {
    let config = s.config.borrow();
    let Some(source) = config.source(name) else {
        return;
    };
    *f.editing.borrow_mut() = Some(source.name.clone());
    f.title.set_text(&source.name);
    f.name.set_text(&source.name);
    for entry in [&f.server, &f.username, &f.password_command, &f.location] {
        entry.set_text("");
    }
    f.password.set_text("");
    match &source.kind {
        SourceKind::Xtream {
            server,
            username,
            password,
            password_command,
        } => {
            f.kind.set_selected(0);
            f.server.set_text(server);
            f.username.set_text(username);
            f.password.set_text(password.as_deref().unwrap_or_default());
            f.password_command
                .set_text(password_command.as_deref().unwrap_or_default());
        }
        SourceKind::M3u { location } => {
            f.kind.set_selected(1);
            f.location.set_text(location);
        }
    }
    f.max_streams
        .set_value(f64::from(source.max_streams.unwrap_or(0)));
    f.limit_mode.set_selected(match source.limit_mode {
        LimitMode::Confirm => 0,
        LimitMode::Block => 1,
    });
    f.remove.set_sensitive(true);
    f.make_default.set_sensitive(
        config
            .sources
            .first()
            .is_some_and(|c| c.name != source.name),
    );
    set_status(f, "", false);
    show_kind_rows(f);
}

fn show_kind_rows(f: &F) {
    let xtream = f.kind.selected() == 0;
    for w in &f.xtream_rows {
        w.set_visible(xtream);
    }
    for w in &f.m3u_rows {
        w.set_visible(!xtream);
    }
}

/// Build a `SourceConfig` from the form (trimmed, empty fields → `None`).
fn read_form(f: &F) -> SourceConfig {
    let text = |e: &gtk::Entry| e.text().trim().to_string();
    let optional = |v: String| (!v.is_empty()).then_some(v);
    let kind = if f.kind.selected() == 0 {
        SourceKind::Xtream {
            server: text(&f.server),
            username: text(&f.username),
            // Passwords are taken verbatim: spaces may be significant.
            password: optional(f.password.text().to_string()),
            password_command: optional(text(&f.password_command)),
        }
    } else {
        SourceKind::M3u {
            location: text(&f.location),
        }
    };
    let max = f.max_streams.value_as_int();
    SourceConfig {
        name: text(&f.name),
        max_streams: (max > 0).then_some(max as u32),
        limit_mode: if f.limit_mode.selected() == 1 {
            LimitMode::Block
        } else {
            LimitMode::Confirm
        },
        kind,
    }
}

fn save(s: &S, f: &F) {
    let source = read_form(f);
    let name = source.name.clone();
    let original = f.editing.borrow().clone();
    if let Err(e) = s.config.borrow_mut().upsert(original.as_deref(), source) {
        set_status(f, &format!("{e:#}"), true);
        return;
    }
    if let Err(e) = window::save_config(s) {
        set_status(f, &format!("Could not save: {e}"), true);
        return;
    }
    refresh_list(s, f);
    select(f, &name);
    set_status(f, "Saved.", false);

    // Reopen if the edited account is the one on screen (credentials or
    // limits may have changed), or open it if nothing is open yet.
    let current = s.current.borrow().clone();
    let was_open = current.is_some() && current == original;
    let nothing_open = current.is_none() && s.config.borrow().sources.len() == 1;
    if was_open || nothing_open {
        window::open_named(s, &name);
    } else {
        window::sync_sources(s);
    }
}

async fn remove(s: &S, f: &F) {
    let Some(name) = f.editing.borrow().clone() else {
        return;
    };
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(format!("Remove {name}?"))
        .detail("The account is deleted from config.toml. A backup of the previous file is kept as config.toml.bak.")
        .buttons(["Cancel", "Remove"])
        .cancel_button(0)
        .default_button(0)
        .build();
    if !matches!(dialog.choose_future(Some(&f.window)).await, Ok(1)) {
        return;
    }
    s.config.borrow_mut().remove(&name);
    if let Err(e) = window::save_config(s) {
        set_status(f, &format!("Could not save: {e}"), true);
        return;
    }
    refresh_list(s, f);
    let first = s.config.borrow().sources.first().map(|c| c.name.clone());
    match &first {
        Some(first) => select(f, first),
        None => start_new(f),
    }
    set_status(f, &format!("Removed {name}."), false);

    if s.current.borrow().as_deref() == Some(name.as_str()) {
        match first {
            Some(first) => window::open_named(s, &first),
            None => window::show_empty(s),
        }
    } else {
        window::sync_sources(s);
    }
}

/// Log in (or fetch the playlist) without saving, on a worker thread.
fn test_login(f: &F) {
    let source = read_form(f);
    if let Err(e) = source.validate() {
        set_status(f, &format!("{e:#}"), true);
        return;
    }
    set_status(f, "Testing…", false);
    f.test.set_sensitive(false);
    let f = f.clone();
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || {
            let resolved = source.resolve()?;
            resolved.source.load().map(|loaded| loaded.summary)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("test thread panicked")));
        f.test.set_sensitive(true);
        match result {
            Ok(summary) => set_status(&f, &format!("✓ Works: {summary}"), false),
            Err(e) => set_status(&f, &format!("✗ {e:#}"), true),
        }
    });
}

fn set_status(f: &F, text: &str, error: bool) {
    f.status.set_text(text);
    if error {
        f.status.add_css_class("error");
    } else {
        f.status.remove_css_class("error");
    }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .build()
}
