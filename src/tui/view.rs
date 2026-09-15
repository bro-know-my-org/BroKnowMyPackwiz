use super::{
    app::{App, PAGES, TYPES},
    i18n::Language,
};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Clear, Paragraph, Row, Table, Wrap},
};

pub(super) type Action = (KeyCode, &'static str, &'static str);
const CLOSE: &[Action] = &[(KeyCode::Esc, "Esc", "close")];
const QUIT: &[Action] = &[
    (KeyCode::Char('w'), "W", "wait_exit"),
    (KeyCode::Char('c'), "C", "cancel_rollback"),
    (KeyCode::Esc, "Esc", "back"),
];

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let lang = app.language;
    app.table_area = Rect::default();
    app.tabs.clear();
    app.buttons.clear();
    app.menu_area = Rect::default();
    if area.width < 40 || area.height < 10 {
        frame.render_widget(Paragraph::new(lang.text("small")), area);
        return;
    }
    let bands = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(toolbar_height(app, area.width)),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!("bkmpw {} · {}", crate::VERSION, app.root.display())),
        bands[0],
    );
    draw_buttons(
        frame,
        lang,
        &[
            (KeyCode::Char('l'), "L", "language"),
            (KeyCode::Char('?'), "?", "help"),
            (KeyCode::Char('q'), "Q", "quit"),
        ],
        Rect::new(bands[0].x, bands[0].y + 1, bands[0].width, 1),
        &mut app.buttons,
    );
    app.tabs = Layout::horizontal([Constraint::Ratio(1, 5); 5])
        .split(bands[1])
        .to_vec();
    for (i, rect) in app.tabs.iter().enumerate() {
        frame.render_widget(
            Paragraph::new(lang.text(PAGES[i]))
                .block(Block::bordered())
                .style(if app.page == i {
                    active()
                } else {
                    Style::default()
                }),
            *rect,
        );
    }
    if app.page == 4 {
        app.menu_area = Block::bordered().inner(bands[2]);
        let items: Vec<_> = super::dialog::SETTINGS
            .iter()
            .map(|key| ratatui::widgets::ListItem::new(lang.text(key)))
            .collect();
        let mut selection = ratatui::widgets::ListState::default()
            .with_selected(Some(app.settings_index))
            .with_offset(app.settings_offset);
        frame.render_stateful_widget(
            ratatui::widgets::List::new(items)
                .highlight_symbol("› ")
                .block(Block::bordered().title(lang.text("settings"))),
            bands[2],
            &mut selection,
        );
        app.settings_offset = selection.offset();
    } else if app.page == 3 {
        super::jobs::draw(frame, &mut app.jobs, lang, bands[2]);
    } else if app.page == 0 {
        files(frame, app, bands[2]);
    } else if app.page == 1 {
        app.catalog.draw(frame, bands[2], lang);
    } else {
        app.menu_area = Block::bordered().inner(bands[2]);
        let items: Vec<_> = super::pack::ACTIONS
            .iter()
            .map(|kind| ratatui::widgets::ListItem::new(lang.text(kind.command())))
            .collect();
        frame.render_stateful_widget(
            ratatui::widgets::List::new(items)
                .highlight_symbol("› ")
                .block(Block::bordered().title(lang.text(
                    if app.root.join("pack.toml").is_file() {
                        "pack"
                    } else {
                        "no_pack"
                    },
                ))),
            bands[2],
            &mut app.pack_selection,
        );
    }
    toolbar(frame, app, bands[3]);
    let popup_text = if app.jobs.quit_prompt {
        lang.text("quit_task")
    } else {
        app.error.as_deref().unwrap_or_default()
    };
    if app.popup_text != popup_text {
        app.popup_scroll = 0;
        app.popup_text = popup_text.to_owned();
    }
    if let Some(dialog) = &mut app.dialog {
        dialog.form.draw(frame, lang);
    } else if app.jobs.quit_prompt {
        app.buttons.clear();
        popup(
            frame,
            lang,
            lang.text("tasks"),
            lang.text("quit_task"),
            QUIT,
            &mut app.popup_scroll,
            &mut app.buttons,
        );
    } else if app.help {
        app.buttons.clear();
        help(frame, lang, &mut app.help_scroll, &mut app.buttons);
    } else if let Some(error) = &app.error {
        app.buttons.clear();
        popup(
            frame,
            lang,
            lang.text("error"),
            error,
            CLOSE,
            &mut app.popup_scroll,
            &mut app.buttons,
        );
    }
}

fn actions(app: &App) -> Vec<(crossterm::event::KeyCode, &'static str, &'static str)> {
    use crossterm::event::KeyCode;
    match app.page {
        0 if app.adding.busy() => vec![(KeyCode::Char('c'), "C", "cf_cancel_preview")],
        0 => vec![
            (KeyCode::Char('u'), "U", "update_preview"),
            (KeyCode::Delete, "Del", "remove"),
            (KeyCode::Char('d'), "D", "update_direct"),
            (KeyCode::Char('a'), "A", "select_all"),
            (KeyCode::Char('e'), "E", "edit_metadata"),
            (KeyCode::Char('p'), "P", "pin"),
            (KeyCode::Char('E'), "Shift+E", "advanced"),
        ],
        1 if app.adding.busy() => vec![(KeyCode::Char('c'), "C", "cf_cancel_preview")],
        1 => vec![
            (KeyCode::Char('g'), "G", "github_repository"),
            (KeyCode::Char('u'), "U", "source_url"),
            (KeyCode::Char('a'), "A", "source_local"),
        ],
        3 => vec![
            (KeyCode::Char(' '), "Space", "resume_pause"),
            (KeyCode::Char('c'), "C", "cancel"),
            (KeyCode::Char('r'), "R", "retry_recovery"),
            (KeyCode::Char('k'), "K", "keep_short"),
            (KeyCode::Char('o'), "O", "restore_short"),
        ],
        2 => vec![(KeyCode::Enter, "Enter", "command_open")],
        4 => vec![
            (KeyCode::Enter, "Enter", "edit"),
            (KeyCode::Char('E'), "Shift+E", "advanced"),
        ],
        _ => Vec::new(),
    }
}

fn toolbar_height(app: &App, width: u16) -> u16 {
    button_rows(&actions(app), app.language, width) + 1
}

pub(super) fn button_rows(actions: &[Action], lang: Language, width: u16) -> u16 {
    let mut rows = 1;
    let mut used = 0;
    for (_, key, label) in actions {
        let text = format!(" {key} {} ", lang.text(label));
        let size = (unicode_width::UnicodeWidthStr::width(text.as_str()) as u16).min(width);
        if used > 0 && used + size > width {
            rows += 1;
            used = 0;
        }
        used += size;
    }
    rows
}

fn toolbar(frame: &mut Frame, app: &mut App, area: Rect) {
    draw_buttons(
        frame,
        app.language,
        &actions(app),
        Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1)),
        &mut app.buttons,
    );
    frame.render_widget(
        Paragraph::new(app.language.text("keys")),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
}

pub(super) fn draw_buttons(
    frame: &mut Frame,
    lang: Language,
    actions: &[Action],
    area: Rect,
    buttons: &mut Vec<(KeyCode, Rect)>,
) {
    let mut x = area.x;
    let mut y = area.y;
    for (code, key, label) in actions {
        let text = format!(" {key} {} ", lang.text(label));
        let width = (unicode_width::UnicodeWidthStr::width(text.as_str()) as u16).min(area.width);
        if x > area.x && x.saturating_add(width) > area.right() {
            x = area.x;
            y += 1;
        }
        if y >= area.bottom() {
            break;
        }
        let rect = Rect::new(x, y, width, 1);
        frame.render_widget(Paragraph::new(text).style(active()), rect);
        buttons.push((*code, rect));
        x += width;
    }
}

fn files(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.language;
    let controls = [
        (KeyCode::Char('f'), "F", "cf_type"),
        (KeyCode::Char('s'), "S", "sort"),
        (KeyCode::Enter, "Enter", "details"),
        (KeyCode::Char('r'), "R", "reload_files"),
    ];
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(button_rows(&controls, lang, area.width)),
        Constraint::Min(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!(
            "{}: {}  [{}]",
            lang.text("search"),
            app.filter,
            lang.text(TYPES[app.kind])
        ))
        .block(Block::bordered())
        .style(if app.editing {
            active()
        } else {
            Style::default()
        }),
        rows[0],
    );
    app.buttons.push((KeyCode::Char('/'), rows[0]));
    draw_buttons(frame, lang, &controls, rows[1], &mut app.buttons);
    let panes = if rows[2].width >= 95 {
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(rows[2])
            .to_vec()
    } else {
        vec![rows[2]]
    };
    if panes.len() == 1 && app.detail {
        details(frame, app, panes[0]);
        return;
    }
    let indices = app.visible();
    let title = if app.loading() {
        lang.text("loading").to_string()
    } else {
        format!(
            "{} · {} / {}",
            lang.text("files"),
            indices.len(),
            app.marked.len()
        )
    };
    let table_rows: Vec<Row> = indices
        .iter()
        .map(|i| {
            let entry = &app.entries[*i];
            Row::new(vec![
                if app.marked.contains(&entry.path) {
                    "●".into()
                } else {
                    " ".into()
                },
                entry.name.clone(),
                entry.source.clone(),
                lang.text(if entry.present {
                    "installed"
                } else {
                    "missing"
                })
                .to_string(),
            ])
        })
        .collect();
    let header = Row::new(["", lang.text("name"), lang.text("source"), ""]);
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::bordered().title(title))
    .row_highlight_style(active());
    let inner = Block::bordered().inner(panes[0]);
    app.table_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );
    frame.render_stateful_widget(table, panes[0], &mut app.table);
    if indices.is_empty() && !app.loading() {
        frame.render_widget(Paragraph::new(lang.text("empty")), app.table_area);
    }
    if panes.len() == 2 {
        details(frame, app, panes[1]);
    }
}

fn details(frame: &mut Frame, app: &App, area: Rect) {
    let lang = app.language;
    let text = app
        .current()
        .map(|entry| {
            let meta = &entry.metadata;
            let fields = [
                ("name", entry.name.clone()),
                ("path", entry.path.clone()),
                ("source", entry.source.clone()),
                ("side", entry.side.clone()),
                ("filename", meta.filename.clone().unwrap_or_default()),
                ("pin", boolean(lang, meta.pin)),
                ("preserve", boolean(lang, meta.preserve)),
            ];
            fields
                .iter()
                .map(|(key, value)| format!("{}: {value}", lang.text(key)))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_else(|| lang.text("empty").to_string());
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(lang.text("details"))),
        area,
    );
}

fn boolean(lang: Language, value: bool) -> String {
    lang.text(if value { "yes" } else { "no" }).into()
}
fn active() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn popup(
    frame: &mut Frame,
    lang: Language,
    title: &str,
    text: &str,
    actions: &[Action],
    scroll: &mut u16,
    buttons: &mut Vec<(KeyCode, Rect)>,
) {
    let area = frame.area();
    let rect = Rect::new(
        area.x + area.width / 10,
        area.y + area.height / 10,
        area.width * 8 / 10,
        area.height * 8 / 10,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(title)
        .title_bottom(lang.text("popup_scroll"));
    let inner = block.inner(rect);
    let rows = button_rows(actions, lang, inner.width).min(inner.height);
    frame.render_widget(block, rect);
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(rows),
    );
    scroll_text(frame, text, body, scroll);
    draw_buttons(
        frame,
        lang,
        actions,
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(rows),
            inner.width,
            rows,
        ),
        buttons,
    );
}

fn help(frame: &mut Frame, lang: Language, scroll: &mut u16, buttons: &mut Vec<(KeyCode, Rect)>) {
    let area = frame.area();
    let rect = Rect::new(
        area.x + area.width / 10,
        area.y + area.height / 10,
        area.width * 8 / 10,
        area.height * 8 / 10,
    );
    let block = Block::bordered().title(lang.text("help"));
    let inner = block.inner(rect);
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    scroll_text(frame, lang.text("help_text"), body, scroll);
    draw_buttons(
        frame,
        lang,
        CLOSE,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        buttons,
    );
}

fn scroll_text(frame: &mut Frame, text: &str, body: Rect, scroll: &mut u16) {
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        let mut width = 0;
        for character in line.chars() {
            let size = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0) as u16;
            if width > 0 && width + size > body.width {
                lines.push(std::mem::take(&mut current));
                width = 0;
            }
            current.push(character);
            width += size;
        }
        lines.push(current);
    }
    *scroll = (*scroll).min(
        lines
            .len()
            .saturating_sub(body.height as usize)
            .min(u16::MAX as usize) as u16,
    );
    frame.render_widget(Paragraph::new(lines.join("\n")).scroll((*scroll, 0)), body);
}
