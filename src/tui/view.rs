use super::{
    app::{App, PAGES, TYPES},
    i18n::Language,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Clear, Paragraph, Row, Table, Wrap},
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let lang = app.language;
    app.table_area = Rect::default();
    app.tabs.clear();
    if area.width < 40 || area.height < 10 {
        frame.render_widget(Paragraph::new(lang.text("small")), area);
        return;
    }
    let bands = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!("bkmpw {} · {}", crate::VERSION, app.root.display())),
        bands[0],
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
    if app.page == 0 {
        files(frame, app, bands[2]);
    } else {
        frame.render_widget(
            Paragraph::new(lang.text("coming"))
                .block(Block::bordered().title(lang.text(PAGES[app.page]))),
            bands[2],
        );
    }
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n{}",
            lang.text("keys"),
            lang.text("filter_keys")
        )),
        bands[3],
    );
    if app.help {
        popup(frame, lang.text("help"), lang.text("help_text"));
    } else if let Some(error) = &app.error {
        popup(frame, lang.text("error"), error);
    }
}

fn files(frame: &mut Frame, app: &mut App, area: Rect) {
    let lang = app.language;
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
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
    let panes = if rows[1].width >= 95 {
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(rows[1])
            .to_vec()
    } else {
        vec![rows[1]]
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

pub fn popup(frame: &mut Frame, title: &str, text: &str) {
    let area = frame.area();
    let rect = Rect::new(
        area.x + area.width / 10,
        area.y + area.height / 10,
        area.width * 8 / 10,
        area.height * 8 / 10,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(title)),
        rect,
    );
}
