//! Rendering. Uses the terminal's own palette (no hard-coded colours) so the
//! UI follows the terminal theme.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph, Wrap};

use crate::app::{App, Focus, Mode, Section, Selected};

const HINTS: &str = "1-3 section · j/k move · h/l pane · / search · a accounts · i info · u url · enter play · space pause · s stop · q quit";
const DETAILS_HEIGHT: u16 = 10;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn spinner_frame(elapsed: std::time::Duration) -> char {
    SPINNER[(elapsed.as_millis() / 100 % SPINNER.len() as u128) as usize]
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).areas(frame.area());
    let [groups, right] =
        Layout::horizontal([Constraint::Percentage(25), Constraint::Percentage(75)]).areas(main);
    let (channels, details) = if app.show_details {
        let [list, details] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(DETAILS_HEIGHT)]).areas(right);
        (list, Some(details))
    } else {
        (right, None)
    };

    app.groups_area = groups;
    app.channels_area = channels;
    let highlight = Style::new().add_modifier(Modifier::REVERSED);

    let groups_title = match (app.section(), app.open_series()) {
        (_, Some(_)) => " Seasons ",
        (Section::Live, _) => " Groups ",
        _ => " Categories ",
    };
    // After a change the lists wipe in from the top: rows past the revealed
    // edge are blank for a few frames.
    let progress = app.reveal();
    let group_edge = app.groups_state.offset() + revealed(groups, progress);
    let group_list = List::new(
        app.group_names()
            .iter()
            .enumerate()
            .map(|(i, g)| ListItem::new(if i < group_edge { g.as_str() } else { "" })),
    )
    .block(pane(groups_title, app.focus == Focus::Groups))
    .highlight_style(highlight);
    // The lists borrow `app`, so render with a copy of the state and store it back.
    let mut groups_state = app.groups_state.clone();
    frame.render_stateful_widget(group_list, groups, &mut groups_state);
    app.groups_state = groups_state;

    let row_edge = app.channels_state.offset() + revealed(channels, progress);
    let items: Vec<ListItem> = app
        .rows()
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            if i >= row_edge {
                return ListItem::new("");
            }
            let marker = if row.playing { "▶ " } else { "  " };
            ListItem::new(format!("{marker}{}", row.name))
        })
        .collect();
    let list_name = match (app.section(), app.open_series()) {
        (_, Some(series)) => series.to_string(),
        (Section::Live, _) => "Channels".to_string(),
        (Section::Movies, _) => "Movies".to_string(),
        (Section::Series, _) => "Series".to_string(),
    };
    let mut title = format!(" {list_name} ({}) ", app.visible_len());
    if let Some(name) = &app.current_account {
        title.push_str(&format!("· {name} "));
    }
    if app.mode == Mode::Search || !app.query.is_empty() {
        let cursor = if app.mode == Mode::Search { "_" } else { "" };
        title.push_str(&format!("/{}{cursor} ", app.query));
    }
    let channel_list = List::new(items)
        .block(pane(&title, app.focus == Focus::Channels))
        .highlight_style(highlight);
    let mut channels_state = app.channels_state.clone();
    frame.render_stateful_widget(channel_list, channels, &mut channels_state);
    app.channels_state = channels_state;

    if let Some(area) = details {
        draw_details(frame, app, area);
    }

    let left = match app.now_playing() {
        Some(c) => format!(" ▶ {} · {}", c.name, app.status),
        None => format!(" {}", app.status),
    };
    let [status_area, hints_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(status);
    // At most half the line, so the status stays readable: drop the update
    // step, then the key hint, when they don't fit.
    let update = app.update.as_ref().and_then(|u| {
        let room = usize::from(status_area.width / 2);
        [
            format!("{u} · x dismiss "),
            format!("{} · x dismiss ", u.headline()),
            format!("{} ", u.headline()),
        ]
        .into_iter()
        .find(|text| text.chars().count() <= room)
    });
    let width = update.as_deref().map_or(0, |u| u.chars().count() as u16);
    let [status_area, update_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(status_area);
    frame.render_widget(Line::from(left).bold(), status_area);
    if let Some(update) = update {
        frame.render_widget(Line::from(update).reversed(), update_area);
    }
    let hints = if matches!(app.mode, Mode::Loading { .. }) {
        "q / Ctrl-C quit"
    } else {
        HINTS
    };
    frame.render_widget(Line::from(hints).dim(), hints_area);

    if let Mode::Loading {
        title,
        name,
        started,
    } = &app.mode
    {
        draw_loading(frame, title, name, started.elapsed());
    }
    if app.mode == Mode::Accounts {
        draw_accounts(frame, app);
    }
    if let Mode::Alert {
        title,
        message,
        pending,
    } = &app.mode
    {
        draw_alert(frame, title, message, pending.is_some());
    }
}

/// How many rows of a bordered list in `area` are revealed at `progress`
/// (0.0 to 1.0). Everything once the transition is over.
fn revealed(area: Rect, progress: f32) -> usize {
    if progress >= 1.0 {
        return usize::MAX;
    }
    let rows = f32::from(area.height.saturating_sub(2));
    (rows * progress).ceil() as usize
}

fn draw_loading(frame: &mut Frame, title: &str, name: &str, elapsed: std::time::Duration) {
    let [area] = Layout::horizontal([Constraint::Length(
        frame.area().width.saturating_sub(2).min(64),
    )])
    .flex(Flex::Center)
    .areas(frame.area());
    let [area] = Layout::vertical([Constraint::Length(5.min(frame.area().height))])
        .flex(Flex::Center)
        .areas(area);
    let spinner = spinner_frame(elapsed);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {title} ").bold())
        .title_bottom(Line::from(" q / Ctrl-C quit ").centered());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(format!("\n{spinner} Loading {name}…"))
            .centered()
            .wrap(Wrap { trim: true })
            .block(block),
        area,
    );
}

fn draw_accounts(frame: &mut Frame, app: &mut App) {
    let width = frame.area().width.saturating_sub(2).min(64);
    let height = (app.accounts().len().max(3).min(u16::MAX as usize - 2) as u16 + 2)
        .min(frame.area().height);
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(frame.area());
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .title(" Accounts ".bold())
        .title_bottom(Line::from(" j/k move · enter switch · esc cancel ").centered());
    frame.render_widget(Clear, area);
    if app.accounts().is_empty() {
        frame.render_widget(
            Paragraph::new("No accounts configured. Add [[source]] entries to your config file.")
                .wrap(Wrap { trim: true })
                .block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = app
        .accounts()
        .iter()
        .map(|name| {
            let marker = if app.current_account.as_ref() == Some(name) {
                "● "
            } else {
                "  "
            };
            ListItem::new(format!("{marker}{name}"))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, area, &mut app.accounts_state);
}

fn draw_details(frame: &mut Frame, app: &App, area: Rect) {
    let block = pane(" Details ", false);
    let lines = match app.selected() {
        None => {
            frame.render_widget(Paragraph::new("Nothing selected").block(block), area);
            return;
        }
        Some(Selected::Channel(c)) => {
            let mut lines = vec![
                Line::from(vec![
                    Span::from(c.name.as_str()).bold(),
                    Span::from(if app.is_playing(c) {
                        "  ▶ playing"
                    } else {
                        ""
                    }),
                ]),
                field("Group", c.group.as_deref().unwrap_or("–")),
            ];
            if app.section() == Section::Live {
                lines.push(field(
                    "Number",
                    &c.number.map_or_else(|| "–".to_string(), |n| n.to_string()),
                ));
                lines.push(field(
                    "Catch-up",
                    &c.catchup_days
                        .map_or_else(|| "none".to_string(), |d| format!("{d} days")),
                ));
                lines.push(field("EPG id", c.tvg_id.as_deref().unwrap_or("–")));
            }
            lines.push(field("Format", c.stream_format()));
            // Host only: stream URLs often embed the account password.
            lines.push(field("Server", tuner_app::source::host(&c.url)));
            lines.push(field("Logo", if c.logo.is_some() { "yes" } else { "no" }));
            // Full URL only on request (`u`): it usually embeds the password.
            lines.push(if app.show_url {
                field("URL", &c.url)
            } else {
                field("URL", "hidden · press u to show")
            });
            lines
        }
        Some(Selected::Series(s)) => {
            let mut lines = vec![
                Line::from(Span::from(s.name.as_str()).bold()),
                field("Category", s.group.as_deref().unwrap_or("–")),
                field("Year", s.year.as_deref().unwrap_or("–")),
                field("Rating", s.rating.as_deref().unwrap_or("–")),
            ];
            if let Some(plot) = &s.plot {
                lines.push(Line::from(plot.as_str()));
            }
            lines
        }
    };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(block),
        area,
    );
}

fn field<'a>(label: &'a str, value: &str) -> Line<'a> {
    Line::from(vec![
        Span::from(format!("{label:<10}")).dim(),
        Span::from(value.to_string()),
    ])
}

fn draw_alert(frame: &mut Frame, title: &str, message: &str, confirm: bool) {
    let width = frame.area().width.saturating_sub(4).min(72);
    let footer = if confirm {
        "y  play anyway        any other key  cancel"
    } else {
        "press any key"
    };
    let text_width = width.saturating_sub(4).max(1) as usize;
    let body_lines = message.len().div_ceil(text_width) as u16 + 1;
    let height = (body_lines + 4).min(frame.area().height);

    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(frame.area());
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);

    let block = Block::bordered()
        .border_type(BorderType::Double)
        .title(format!(" ⚠ {title} ").bold())
        .title_bottom(Line::from(format!(" {footer} ")).centered());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(message)
            .wrap(Wrap { trim: true })
            .block(block.padding(ratatui::widgets::Padding::horizontal(1))),
        area,
    );
}

fn pane(title: &str, focused: bool) -> Block<'_> {
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded);
    if focused { block } else { block.dim() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reveal_grows_from_nothing_to_every_row() {
        let area = Rect::new(0, 0, 20, 12); // 10 rows inside the border
        assert_eq!(revealed(area, 0.0), 0);
        assert_eq!(revealed(area, 0.25), 3);
        assert_eq!(revealed(area, 0.99), 10);
        assert_eq!(revealed(area, 1.0), usize::MAX);
        assert_eq!(revealed(Rect::new(0, 0, 20, 1), 0.5), 0, "no room, no rows");
    }

    #[test]
    fn spinner_advances_every_100ms_and_wraps() {
        assert_eq!(spinner_frame(Duration::ZERO), '⠋');
        assert_eq!(spinner_frame(Duration::from_millis(99)), '⠋');
        assert_eq!(spinner_frame(Duration::from_millis(100)), '⠙');
        assert_eq!(spinner_frame(Duration::from_millis(900)), '⠏');
        assert_eq!(spinner_frame(Duration::from_millis(1000)), '⠋');
    }
}
