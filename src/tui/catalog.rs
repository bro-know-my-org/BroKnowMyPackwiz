use super::i18n::Language;
use crate::{
    catalog::curseforge::{Client, File, Filter, Page, Project},
    operation::{Error, Result},
};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver},
};

enum Results {
    Projects(Page<Project>),
    Files(Page<File>),
}
impl Results {
    fn len(&self) -> usize {
        match self {
            Self::Projects(p) => p.items.len(),
            Self::Files(p) => p.items.len(),
        }
    }
    fn more(&self) -> bool {
        match self {
            Self::Projects(p) => p.has_more(),
            Self::Files(p) => p.has_more(),
        }
    }
}

#[derive(Clone)]
pub struct Selection {
    pub file: File,
    pub relaxed: bool,
    pub class_id: u64,
    pub download: bool,
}
#[derive(Default)]
pub struct Browser {
    pub editing: bool,
    pub query: String,
    pub chosen: Option<Selection>,
    kind: usize,
    relaxed: bool,
    page: usize,
    project: Option<Project>,
    results: Option<Results>,
    selection: ListState,
    pending: Option<Receiver<Result<(Results, Filter)>>>,
    filter: Filter,
    error: Option<Error>,
    area: Rect,
    input: Rect,
    buttons: Vec<(KeyCode, Rect)>,
}
impl Browser {
    fn search(&mut self, root: &Path) {
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        self.results = None;
        self.chosen = None;
        self.selection = ListState::default().with_selected(Some(0));
        let root = root.to_path_buf();
        let query = self.query.clone();
        let project = self.project.as_ref().map(|p| p.id);
        let (page, kind, relaxed) = (self.page, self.kind, self.relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = (|| {
                let client = Client::for_pack(&root)?;
                let mut filter = if relaxed {
                    Filter::default()
                } else {
                    Filter::for_pack(&root)?
                };
                if kind != 0 {
                    filter.loader = None;
                }
                let results = if let Some(id) = project {
                    let mut files = client.files(id, &filter, page)?;
                    // Treat the server's filters as advisory; verify locally too.
                    files.items.retain(|f| f.compatible(&filter));
                    Results::Files(files)
                } else {
                    Results::Projects(client.search(&query, &filter, [6, 12, 6552][kind], page)?)
                };
                Ok((results, filter))
            })();
            let _ = tx.send(result);
        });
    }
    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(Error::new(
                crate::operation::ErrorCode::Failed,
                "catalog_worker_disconnected",
            )),
        };
        self.pending = None;
        match result {
            Ok((results, filter)) => {
                self.results = Some(results);
                self.filter = filter;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn move_by(&mut self, amount: isize) {
        let count = self.results.as_ref().map_or(0, Results::len);
        self.selection.select((count > 0).then(|| {
            self.selection
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(amount)
                .min(count - 1)
        }));
    }
    pub fn event(&mut self, event: Event, root: &Path) {
        if let Event::Paste(text) = &event {
            if self.editing {
                self.query.extend(text.chars().filter(|c| !c.is_control()));
            }
        }
        if let Event::Mouse(mouse) = &event {
            let point = (mouse.column, mouse.row).into();
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                if let Some((code, _)) = self.buttons.iter().find(|(_, area)| area.contains(point))
                {
                    self.key(*code, root);
                    return;
                }
                if self.input.contains(point) {
                    self.editing = true;
                    return;
                }
            }
            if self.area.contains(point) {
                match mouse.kind {
                    MouseEventKind::ScrollDown => self.move_by(1),
                    MouseEventKind::ScrollUp => self.move_by(-1),
                    MouseEventKind::Down(MouseButton::Left) => {
                        let index = self.selection.offset() + (mouse.row - self.area.y) as usize;
                        if index < self.results.as_ref().map_or(0, Results::len) {
                            self.selection.select(Some(index));
                        }
                    }
                    MouseEventKind::Down(MouseButton::Right) => self.key(KeyCode::Enter, root),
                    _ => {}
                }
            }
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return;
            }
            if self.editing {
                match key.code {
                    KeyCode::Esc => self.editing = false,
                    KeyCode::Enter => {
                        self.editing = false;
                        if self.pending.is_none() {
                            self.project = None;
                            self.page = 0;
                            self.search(root);
                        }
                    }
                    KeyCode::Backspace => {
                        self.query.pop();
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.query.push(c)
                    }
                    _ => {}
                }
            } else {
                self.key(key.code, root);
            }
        }
    }
    fn key(&mut self, code: KeyCode, root: &Path) {
        if code == KeyCode::Char('/') {
            self.editing = true;
            return;
        }
        if self.pending.is_some() {
            return;
        }
        match code {
            KeyCode::Char('r') => self.search(root),
            KeyCode::Char('f') => {
                self.kind = (self.kind + 1) % 3;
                self.project = None;
                self.page = 0;
                self.search(root);
            }
            KeyCode::Char('v') => {
                self.relaxed = !self.relaxed;
                self.page = 0;
                self.search(root);
            }
            KeyCode::Right if self.results.as_ref().is_some_and(Results::more) => {
                self.page += 1;
                self.search(root);
            }
            KeyCode::Left if self.page > 0 => {
                self.page -= 1;
                self.search(root);
            }
            KeyCode::Down => self.move_by(1),
            KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::Esc => {
                self.project = None;
                self.page = 0;
                self.search(root);
            }
            KeyCode::Enter => {
                let index = self.selection.selected().unwrap_or(0);
                match &self.results {
                    Some(Results::Projects(page)) => {
                        if let Some(project) = page.items.get(index) {
                            self.project = Some(project.clone());
                            self.page = 0;
                            self.search(root);
                        }
                    }
                    Some(Results::Files(page)) => {
                        if let Some(file) = page.items.get(index) {
                            self.chosen = Some(Selection {
                                file: file.clone(),
                                relaxed: self.relaxed,
                                class_id: self.project.as_ref().map_or(6, |p| p.class_id),
                                download: false,
                            });
                        }
                    }
                    None => self.search(root),
                }
            }
            _ => {}
        }
    }
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, lang: Language) {
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);
        self.input = rows[0];
        frame.render_widget(
            Paragraph::new(format!(
                "{}: {} · {} · {}",
                lang.text("search"),
                self.query,
                lang.text(["mods", "resourcepacks", "shaderpacks"][self.kind]),
                lang.text(if self.relaxed {
                    "cf_relaxed"
                } else {
                    "cf_compatible"
                })
            ))
            .block(Block::bordered().title("CurseForge"))
            .style(if self.editing {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            }),
            rows[0],
        );
        let panes = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(rows[1]);
        let index = self.selection.selected().unwrap_or(0);
        let (items, detail): (Vec<ListItem>, String) = match &self.results {
            Some(Results::Projects(page)) => (
                page.items
                    .iter()
                    .map(|p| ListItem::new(p.name.clone()))
                    .collect(),
                page.items
                    .get(index)
                    .map(|p| format!("{}\n\n{}\n\n{}", p.name, p.summary, p.website))
                    .unwrap_or_default(),
            ),
            Some(Results::Files(page)) => (
                page.items
                    .iter()
                    .map(|f| ListItem::new(format!("{} · {}", f.name, f.id)))
                    .collect(),
                page.items
                    .get(index)
                    .map(|f| {
                        format!(
                            "{}\n{}\n{}\n{} bytes\nSHA-1: {}",
                            f.filename,
                            f.versions.join(", "),
                            f.date,
                            f.size,
                            f.sha1
                        )
                    })
                    .unwrap_or_default(),
            ),
            None => (vec![], String::new()),
        };
        let title = self
            .project
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("CurseForge");
        self.area = Block::bordered().inner(panes[0]);
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::bordered().title(format!("{title} · {}", self.page + 1)))
                .highlight_style(Style::default().fg(Color::Cyan))
                .highlight_symbol("› "),
            panes[0],
            &mut self.selection,
        );
        let detail = if self.pending.is_some() {
            lang.text("loading").into()
        } else if let Some(error) = &self.error {
            lang.error(error)
        } else if detail.is_empty() {
            lang.text("cf_search_hint").into()
        } else {
            detail
        };
        frame.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(lang.text("details"))),
            panes[1],
        );
        self.buttons.clear();
        let actions = [
            (KeyCode::Char('/'), "/", "search"),
            (KeyCode::Enter, "Enter", "cf_choose"),
            (KeyCode::Char('f'), "F", "cf_type"),
            (KeyCode::Char('v'), "V", "cf_filter"),
            (KeyCode::Left, "←", "cf_prev"),
            (KeyCode::Right, "→", "cf_next"),
        ];
        let mut x = rows[2].x;
        let mut y = rows[2].y;
        for (code, key, label) in actions {
            let text = format!(" {key} {} ", lang.text(label));
            let width = unicode_width::UnicodeWidthStr::width(text.as_str()) as u16;
            if x + width > rows[2].right() {
                x = rows[2].x;
                y += 1;
            }
            if y >= rows[2].bottom() {
                break;
            }
            let rect = Rect::new(x, y, width.min(rows[2].width), 1);
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(Color::Cyan)),
                rect,
            );
            self.buttons.push((code, rect));
            x += width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_input_preserves_unicode_without_activating_shortcuts() {
        let mut browser = Browser {
            editing: true,
            ..Browser::default()
        };
        let root = Path::new("/unused");
        browser.event(Event::Paste("中文qvf\n".into()), root);
        browser.event(
            Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::NONE,
            )),
            root,
        );
        assert_eq!(browser.query, "中文qv");
        assert_eq!(browser.kind, 0);
        assert!(!browser.relaxed);
        assert!(browser.pending.is_none());
    }
    #[test]
    fn background_results_render_in_both_languages_and_narrow_windows() {
        let (tx, rx) = mpsc::channel();
        let mut browser = Browser {
            pending: Some(rx),
            ..Browser::default()
        };
        tx.send(Ok((
            Results::Projects(Page {
                items: vec![Project {
                    id: 1,
                    name: "中文项目".into(),
                    summary: "详情".repeat(100),
                    website: String::new(),
                    class_id: 6,
                }],
                total: 1,
                offset: 0,
            }),
            Filter::default(),
        )))
        .unwrap();
        browser.poll();
        assert!(browser.pending.is_none());
        for lang in [Language::ZhCn, Language::En] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 15)).unwrap();
            terminal.draw(|f| browser.draw(f, f.area(), lang)).unwrap();
            assert!(browser.area.width > 0);
        }
    }
}
