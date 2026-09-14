use super::{
    files::{self, Entry},
    i18n::Language,
};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{layout::Rect, widgets::TableState};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

pub const PAGES: [&str; 5] = ["files", "add", "pack", "tasks", "settings"];
pub const TYPES: [&str; 4] = ["all", "mods", "resourcepacks", "shaderpacks"];

pub struct App {
    pub root: PathBuf,
    pub language: Language,
    pub entries: Vec<Entry>,
    pub page: usize,
    pub filter: String,
    pub kind: usize,
    pub descending: bool,
    pub editing: bool,
    pub marked: BTreeSet<String>,
    pub table: TableState,
    pub table_area: Rect,
    pub tabs: Vec<Rect>,
    pub detail: bool,
    pub help: bool,
    pub error: Option<String>,
    pending: Option<Receiver<Result<Vec<Entry>, String>>>,
}

impl App {
    pub fn new(root: PathBuf) -> Self {
        let mut app = Self {
            root,
            language: Language::detect(),
            entries: Vec::new(),
            page: 0,
            filter: String::new(),
            kind: 0,
            descending: false,
            editing: false,
            marked: BTreeSet::new(),
            table: TableState::default(),
            table_area: Rect::default(),
            tabs: Vec::new(),
            detail: false,
            help: false,
            error: None,
            pending: None,
        };
        app.reload();
        app
    }

    pub fn reload(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error = None;
        std::thread::spawn(move || {
            let _ = tx.send(files::load(&root));
        });
    }

    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(entries) => {
                        self.entries = entries;
                        self.marked
                            .retain(|path| self.entries.iter().any(|entry| &entry.path == path));
                        self.reset_cursor();
                    }
                    Err(error) => self.error = Some(error),
                }
                self.pending = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.error = Some("worker disconnected".into());
                self.pending = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn visible(&self) -> Vec<usize> {
        let query = self.filter.to_lowercase();
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                (self.kind == 0 || entry.path.starts_with(&format!("{}/", TYPES[self.kind])))
                    && format!("{} {} {}", entry.name, entry.path, entry.source)
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        rows.sort_by(|a, b| {
            self.entries[*a]
                .name
                .to_lowercase()
                .cmp(&self.entries[*b].name.to_lowercase())
                .then_with(|| self.entries[*a].path.cmp(&self.entries[*b].path))
        });
        if self.descending {
            rows.reverse();
        }
        rows
    }

    pub fn current(&self) -> Option<&Entry> {
        self.visible()
            .get(self.table.selected()?)
            .map(|i| &self.entries[*i])
    }

    fn reset_cursor(&mut self) {
        self.table.select(if self.visible().is_empty() {
            None
        } else {
            Some(0)
        });
        *self.table.offset_mut() = 0;
    }

    fn move_cursor(&mut self, amount: isize) {
        let count = self.visible().len();
        self.table.select(if count == 0 {
            None
        } else {
            Some(
                self.table
                    .selected()
                    .unwrap_or(0)
                    .saturating_add_signed(amount)
                    .min(count - 1),
            )
        });
    }

    fn mark(&mut self) {
        if let Some(path) = self.current().map(|entry| entry.path.clone()) {
            if !self.marked.remove(&path) {
                self.marked.insert(path);
            }
        }
    }

    pub fn event(&mut self, event: Event) -> bool {
        match event {
            Event::Paste(text) if self.editing => {
                self.filter.extend(text.chars().filter(|c| !c.is_control()));
                self.reset_cursor();
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if self.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => self.editing = false,
                        KeyCode::Backspace => {
                            self.filter.pop();
                        }
                        KeyCode::Char(c)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            self.filter.push(c)
                        }
                        _ => {}
                    }
                    self.reset_cursor();
                    return false;
                }
                if self.help {
                    self.help = false;
                    return false;
                }
                match key.code {
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return true;
                    }
                    KeyCode::Tab => self.page = (self.page + 1) % PAGES.len(),
                    KeyCode::BackTab => self.page = (self.page + PAGES.len() - 1) % PAGES.len(),
                    KeyCode::Char('l') => self.language.toggle(),
                    KeyCode::Char('?') => self.help = true,
                    KeyCode::Char('r') => self.reload(),
                    KeyCode::Char('/') if self.page == 0 => self.editing = true,
                    KeyCode::Char('f') if self.page == 0 => {
                        self.kind = (self.kind + 1) % TYPES.len();
                        self.reset_cursor();
                    }
                    KeyCode::Char('s') if self.page == 0 => {
                        self.descending = !self.descending;
                        self.reset_cursor();
                    }
                    KeyCode::Down if self.page == 0 => self.move_cursor(1),
                    KeyCode::Up if self.page == 0 => self.move_cursor(-1),
                    KeyCode::PageDown if self.page == 0 => self.move_cursor(10),
                    KeyCode::PageUp if self.page == 0 => self.move_cursor(-10),
                    KeyCode::Char(' ') if self.page == 0 => self.mark(),
                    KeyCode::Enter if self.page == 0 => self.detail = !self.detail,
                    KeyCode::Esc => {
                        self.detail = false;
                        self.error = None;
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) if !self.help && !self.editing => {
                let point = ratatui::layout::Position::new(mouse.column, mouse.row);
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                    if let Some(page) = self.tabs.iter().position(|rect| rect.contains(point)) {
                        self.page = page;
                    }
                }
                if self.page == 0 && self.table_area.contains(point) {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => self.move_cursor(1),
                        MouseEventKind::ScrollUp => self.move_cursor(-1),
                        MouseEventKind::Down(button) => {
                            let row =
                                self.table.offset() + (mouse.row - self.table_area.y) as usize;
                            if row < self.visible().len() {
                                self.table.select(Some(row));
                                if button == MouseButton::Right {
                                    self.mark();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        false
    }
}
