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
    pub jobs: super::jobs::Jobs,
    pub dialog: Option<super::dialog::Dialog>,
    pub preferences: super::preferences::Preferences,
    pub persist_preferences: bool,
    pub settings_index: usize,
    pub external: Option<String>,
    pub buttons: Vec<(KeyCode, Rect)>,
    pub menu_area: Rect,
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
            jobs: super::jobs::Jobs::default(),
            dialog: None,
            preferences: super::preferences::Preferences::default(),
            persist_preferences: false,
            settings_index: 0,
            external: None,
            buttons: Vec::new(),
            menu_area: Rect::default(),
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
        match self.jobs.poll() {
            Ok(true) => self.reload(),
            Err(error) => self.error = Some(error.to_string()),
            _ => {}
        }
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
        if self.dialog.is_some() {
            self.dialog_event(event);
            return false;
        }
        if let Event::Mouse(mouse) = &event {
            if !self.jobs.quit_prompt
                && !self.help
                && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            {
                if let Some(code) = self
                    .buttons
                    .iter()
                    .find(|(_, area)| area.contains((mouse.column, mouse.row).into()))
                    .map(|(key, _)| *key)
                {
                    return self.event(Event::Key(crossterm::event::KeyEvent::new(
                        code,
                        KeyModifiers::NONE,
                    )));
                }
            }
        }
        match event {
            Event::Paste(text) if self.editing => {
                self.filter.extend(text.chars().filter(|c| !c.is_control()));
                self.reset_cursor();
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if self.jobs.quit_prompt {
                    if let Err(error) = self.jobs.quit_key(key.code) {
                        self.error = Some(error.to_string());
                    }
                    return false;
                }
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
                    KeyCode::Char('q') => return self.quit(),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return self.quit();
                    }
                    KeyCode::Tab => self.page = (self.page + 1) % PAGES.len(),
                    KeyCode::BackTab => self.page = (self.page + PAGES.len() - 1) % PAGES.len(),
                    KeyCode::Char('l') => {
                        self.language.toggle();
                        self.preferences.language = Some(self.language);
                        if self.persist_preferences {
                            if let Err(error) = self.preferences.save() {
                                self.error = Some(error.to_string());
                            }
                        }
                    }
                    KeyCode::Char('?') => self.help = true,
                    KeyCode::Char('r') if self.page != 3 => self.reload(),
                    KeyCode::Char('p') if self.page == 0 => {
                        let selected: Vec<_> = if self.marked.is_empty() {
                            self.current().into_iter().cloned().collect()
                        } else {
                            self.entries
                                .iter()
                                .filter(|e| self.marked.contains(&e.path))
                                .cloned()
                                .collect()
                        };
                        if let Err(error) = self.jobs.pin(&self.root, &selected) {
                            self.error = Some(error.to_string());
                        }
                    }
                    KeyCode::Char('e') if self.page == 0 => {
                        if let Some(entry) = self.current() {
                            match super::dialog::Dialog::metadata(&self.root, &entry.path) {
                                Ok(dialog) => self.dialog = Some(dialog),
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                    }
                    KeyCode::Char('E') if self.page == 0 => {
                        self.external = self.current().map(|entry| entry.path.clone())
                    }
                    KeyCode::Char('E') if self.page == 4 => {
                        self.external = Some(
                            if self.settings_index == 0 {
                                "pack.toml"
                            } else {
                                ".pw/config.toml"
                            }
                            .into(),
                        )
                    }
                    KeyCode::Down if self.page == 4 => {
                        self.settings_index =
                            (self.settings_index + 1).min(super::dialog::SETTINGS.len() - 1)
                    }
                    KeyCode::Up if self.page == 4 => {
                        self.settings_index = self.settings_index.saturating_sub(1)
                    }
                    KeyCode::Enter if self.page == 4 => {
                        match super::dialog::Dialog::settings(
                            &self.root,
                            self.settings_index,
                            &self.preferences,
                        ) {
                            Ok(dialog) => self.dialog = Some(dialog),
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                    KeyCode::Char(code @ ('k' | 'o')) if self.page == 3 => {
                        if let Some(queue) = &self.jobs.queue {
                            match super::dialog::Dialog::conflicts(
                                queue,
                                self.jobs.selected,
                                code == 'k',
                                self.language,
                            ) {
                                Ok(dialog) => self.dialog = Some(dialog),
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                    }
                    code if self.page == 3 && code != KeyCode::Esc => {
                        if let Err(error) = self.jobs.key(code) {
                            self.error = Some(error.to_string());
                        }
                    }
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
                if self.page == 4 && self.menu_area.contains(point) {
                    let index = (mouse.row - self.menu_area.y) as usize;
                    if index < super::dialog::SETTINGS.len()
                        && mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    {
                        self.settings_index = index;
                        return self.event(Event::Key(crossterm::event::KeyEvent::new(
                            KeyCode::Enter,
                            KeyModifiers::NONE,
                        )));
                    }
                }
                if self.page == 3 {
                    self.jobs.mouse(mouse);
                }
            }
            _ => {}
        }
        false
    }

    fn quit(&mut self) -> bool {
        match self.jobs.quit() {
            Ok(exit) => exit,
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    fn dialog_event(&mut self, event: Event) {
        use super::{dialog::Submission, form::Action};
        let mut dialog = self.dialog.take().unwrap();
        match dialog.form.event(event) {
            Action::Cancel => return,
            Action::Continue => {
                self.dialog = Some(dialog);
                return;
            }
            Action::Submit => {}
        }
        let result = (|| -> crate::operation::Result<()> {
            let state = self
                .jobs
                .queue
                .as_ref()
                .map(|q| q.state.clone())
                .unwrap_or(crate::operation::durable::user_state()?);
            match dialog.submit(&state, &self.preferences)? {
                Submission::Resolve { index, keep } => {
                    let queue = self.jobs.queue.as_mut().ok_or_else(|| {
                        crate::operation::Error::new(
                            crate::operation::ErrorCode::Busy,
                            "queue unavailable",
                        )
                    })?;
                    queue.resolve(index, keep)?;
                }
                Submission::Edit(draft) => {
                    let queue = self.jobs.queue.as_mut().ok_or_else(|| {
                        crate::operation::Error::new(
                            crate::operation::ErrorCode::Busy,
                            "queue unavailable",
                        )
                    })?;
                    queue.enqueue(
                        "edit_metadata",
                        crate::operation::queue::Request::Edit(vec![draft]),
                    )?;
                }
                Submission::Preferences(preferences) => {
                    preferences.save()?;
                    self.preferences = preferences;
                    self.language = self.preferences.language.unwrap_or_else(Language::detect);
                }
                Submission::Open(root) => self.switch_root(root)?,
            }
            Ok(())
        })();
        if let Err(error) = result {
            dialog.form.error = Some(error.to_string());
            self.dialog = Some(dialog);
        }
    }

    fn switch_root(&mut self, root: PathBuf) -> crate::operation::Result<()> {
        if self.jobs.queue.as_ref().is_some_and(|q| q.pending()) {
            return Err(crate::operation::Error::new(
                crate::operation::ErrorCode::Busy,
                "finish or cancel the current queue before switching packs",
            ));
        }
        let root = crate::operation::durable::canonical(&root)?;
        if root.exists() && !root.is_dir() {
            return Err(crate::operation::Error::new(
                crate::operation::ErrorCode::Invalid,
                "not a directory",
            ));
        }
        self.preferences.remember(&root)?;
        self.root = root;
        self.pending = None;
        self.entries.clear();
        self.marked.clear();
        self.reset_cursor();
        self.jobs = super::jobs::Jobs::default();
        self.jobs.connect(self.root.clone());
        self.reload();
        self.page = 0;
        Ok(())
    }
}
