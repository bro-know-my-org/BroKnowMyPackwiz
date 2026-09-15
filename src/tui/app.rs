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
    pub help_scroll: u16,
    pub error: Option<String>,
    pub jobs: super::jobs::Jobs,
    pub dialog: Option<super::dialog::Dialog>,
    pub preferences: super::preferences::Preferences,
    pub persist_preferences: bool,
    pub settings_index: usize,
    pub pack_selection: ratatui::widgets::ListState,
    pub external: Option<String>,
    pub buttons: Vec<(KeyCode, Rect)>,
    pub menu_area: Rect,
    pub catalog: super::catalog::Browser,
    pub adding: super::add::Workflow,
    preparation_artifacts: Option<crate::operation::artifacts::Lease>,
    pending: Option<Receiver<Result<Vec<Entry>, String>>>,
}

impl App {
    pub fn new(root: PathBuf) -> Self {
        let initialize = !root.join("pack.toml").is_file();
        let mut app = Self {
            root,
            language: Language::detect(),
            entries: Vec::new(),
            page: if initialize { 2 } else { 0 },
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
            help_scroll: 0,
            error: None,
            jobs: super::jobs::Jobs::default(),
            dialog: None,
            preferences: super::preferences::Preferences::default(),
            persist_preferences: false,
            settings_index: 0,
            pack_selection: ratatui::widgets::ListState::default().with_selected(Some(
                if initialize {
                    super::pack::ACTIONS.len() - 1
                } else {
                    0
                },
            )),
            external: None,
            buttons: Vec::new(),
            menu_area: Rect::default(),
            catalog: super::catalog::Browser::default(),
            adding: super::add::Workflow::default(),
            preparation_artifacts: None,
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
        self.catalog.poll();
        if self.dialog.is_none() && !self.jobs.quit_prompt && !self.help {
            if let Some(super::add::Prepared { result, artifacts }) = self.adding.poll() {
                self.preparation_artifacts = result.is_ok().then_some(artifacts);
                match result {
                    Ok(super::add::Output::UpdatePreview(preview)) => {
                        self.dialog = Some(super::dialog::Dialog::update(preview, self.language))
                    }
                    Ok(super::add::Output::UpdateReady(planned)) => {
                        self.dialog = Some(super::dialog::Dialog::update_prepared(
                            planned,
                            self.language,
                        ))
                    }
                    Ok(super::add::Output::UpdateDirect(planned)) => {
                        self.dialog = Some(super::dialog::Dialog::update_prepared(
                            planned,
                            self.language,
                        ));
                        self.dialog_event(Event::Key(crossterm::event::KeyEvent::new(
                            KeyCode::Enter,
                            KeyModifiers::NONE,
                        )));
                    }
                    Ok(super::add::Output::GitHub(picker, form)) => {
                        self.dialog =
                            Some(super::dialog::Dialog::github(picker, form, self.language))
                    }
                    Ok(super::add::Output::CurseForge(preview)) => {
                        self.dialog = Some(super::dialog::Dialog::prepared(preview, self.language))
                    }
                    Ok(super::add::Output::Source(prepared)) => {
                        self.dialog = Some(super::dialog::Dialog::source_prepared(
                            prepared,
                            self.language,
                        ))
                    }
                    Err(error) => {
                        self.error = Some(format!(
                            "{}\n{}",
                            self.language.text("cf_preview_failed"),
                            self.language.error(&error)
                        ))
                    }
                }
            } else if let Some(selected) = self.catalog.chosen.take() {
                self.dialog = Some(super::dialog::Dialog::curseforge(selected));
            }
        }
        match self.jobs.poll() {
            Ok(true) => self.reload(),
            Err(error) => self.error = Some(self.language.error(&error)),
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
        if self.help {
            if let Event::Mouse(mouse) = &event {
                match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        self.help_scroll = self.help_scroll.saturating_add(3)
                    }
                    MouseEventKind::ScrollUp => {
                        self.help_scroll = self.help_scroll.saturating_sub(3)
                    }
                    _ => {}
                }
                return false;
            }
        }
        if self.page == 1 && self.catalog.editing && !self.help && !self.jobs.quit_prompt {
            self.catalog.event(event, &self.root);
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
                        self.error = Some(self.language.error(&error));
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
                    match key.code {
                        KeyCode::Down => self.help_scroll = self.help_scroll.saturating_add(1),
                        KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                        KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(8),
                        KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(8),
                        KeyCode::Home => self.help_scroll = 0,
                        KeyCode::End => self.help_scroll = u16::MAX,
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?' | 'q') => {
                            self.help = false
                        }
                        _ => {}
                    }
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
                                self.error = Some(self.language.error(&error));
                            }
                        }
                    }
                    KeyCode::Char('?') => {
                        self.help = true;
                        self.help_scroll = 0;
                    }
                    KeyCode::Down if self.page == 2 => {
                        let index = self.pack_selection.selected().unwrap_or(0);
                        self.pack_selection
                            .select(Some((index + 1).min(super::pack::ACTIONS.len() - 1)));
                    }
                    KeyCode::Up if self.page == 2 => {
                        let index = self.pack_selection.selected().unwrap_or(0);
                        self.pack_selection.select(Some(index.saturating_sub(1)));
                    }
                    KeyCode::Enter if self.page == 2 => {
                        let kind =
                            super::pack::ACTIONS[self.pack_selection.selected().unwrap_or(0)];
                        self.dialog = Some(super::dialog::Dialog::command(kind, self.language));
                    }
                    KeyCode::Char('r') if self.page != 3 && self.page != 1 => self.reload(),
                    KeyCode::Char(code @ ('u' | 'd')) if self.page == 0 => {
                        let paths: Vec<_> = if self.marked.is_empty() {
                            self.current().map(|e| e.path.clone()).into_iter().collect()
                        } else {
                            self.marked.iter().cloned().collect()
                        };
                        if !paths.is_empty() {
                            let result = self
                                .jobs
                                .queue
                                .as_ref()
                                .ok_or_else(|| {
                                    crate::operation::Error::named(
                                        crate::operation::ErrorCode::Busy,
                                        "queue_unavailable",
                                        "queue unavailable",
                                    )
                                })
                                .map(|q| q.state.clone())
                                .and_then(|state| {
                                    self.adding.update(&self.root, &state, paths, code == 'd')
                                });
                            if let Err(error) = result {
                                self.error = Some(self.language.error(&error));
                            }
                        }
                    }
                    KeyCode::Char('a') if self.page == 0 => {
                        let paths: Vec<_> = self
                            .visible()
                            .iter()
                            .map(|i| self.entries[*i].path.clone())
                            .collect();
                        if paths.iter().all(|path| self.marked.contains(path)) {
                            for path in paths {
                                self.marked.remove(&path);
                            }
                        } else {
                            self.marked.extend(paths);
                        }
                    }
                    KeyCode::Delete if self.page == 0 => {
                        let names: Vec<_> = if self.marked.is_empty() {
                            self.current()
                                .map(|entry| entry.path.clone())
                                .into_iter()
                                .collect()
                        } else {
                            self.marked.iter().cloned().collect()
                        };
                        if !names.is_empty() {
                            match super::dialog::Dialog::remove(&self.root, names, self.language) {
                                Ok(dialog) => self.dialog = Some(dialog),
                                Err(error) => self.error = Some(self.language.error(&error)),
                            }
                        }
                    }
                    KeyCode::Char('c') if self.page == 0 => self.adding.cancel(),
                    KeyCode::Char('c') if self.page == 1 => self.adding.cancel(),
                    KeyCode::Char('g') if self.page == 1 => {
                        let (picker, form) = super::github::Picker::repository();
                        self.dialog =
                            Some(super::dialog::Dialog::github(picker, form, self.language));
                    }
                    KeyCode::Char(code @ ('u' | 'a')) if self.page == 1 => {
                        self.dialog = Some(super::dialog::Dialog::source(if code == 'u' {
                            crate::catalog::source::Input::Url {
                                url: String::new(),
                                sha256: String::new(),
                            }
                        } else {
                            crate::catalog::source::Input::Local(PathBuf::new())
                        }));
                    }
                    code if self.page == 1 => {
                        if code == KeyCode::Esc {
                            self.error = None;
                        }
                        self.catalog.event(
                            Event::Key(crossterm::event::KeyEvent::new(code, key.modifiers)),
                            &self.root,
                        );
                    }
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
                            self.error = Some(self.language.error(&error));
                        }
                    }
                    KeyCode::Char('e') if self.page == 0 => {
                        if let Some(entry) = self.current() {
                            match super::dialog::Dialog::metadata(&self.root, &entry.path) {
                                Ok(dialog) => self.dialog = Some(dialog),
                                Err(error) => self.error = Some(self.language.error(&error)),
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
                            Err(error) => self.error = Some(self.language.error(&error)),
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
                                Err(error) => self.error = Some(self.language.error(&error)),
                            }
                        }
                    }
                    code if self.page == 3 && code != KeyCode::Esc => {
                        if let Err(error) = self.jobs.key(code) {
                            self.error = Some(self.language.error(&error));
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
                if self.page == 2 && self.menu_area.contains(point) {
                    let code = match mouse.kind {
                        MouseEventKind::ScrollDown => Some(KeyCode::Down),
                        MouseEventKind::ScrollUp => Some(KeyCode::Up),
                        MouseEventKind::Down(MouseButton::Left) => {
                            let index = self.pack_selection.offset()
                                + (mouse.row - self.menu_area.y) as usize;
                            if index < super::pack::ACTIONS.len() {
                                self.pack_selection.select(Some(index));
                                Some(KeyCode::Enter)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if let Some(code) = code {
                        return self.event(Event::Key(crossterm::event::KeyEvent::new(
                            code,
                            KeyModifiers::NONE,
                        )));
                    }
                }
                if self.page == 3 {
                    self.jobs.mouse(mouse);
                }
                if self.page == 1 {
                    self.catalog.event(Event::Mouse(mouse), &self.root);
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
                self.error = Some(self.language.error(&error));
                false
            }
        }
    }

    fn dialog_event(&mut self, event: Event) {
        use super::{dialog::Submission, form::Action};
        let mut dialog = self.dialog.take().unwrap();
        match dialog.form.event(event) {
            Action::Cancel => {
                self.preparation_artifacts.take();
                if let Some(queue) = &self.jobs.queue {
                    dialog.discard(queue);
                }
                return;
            }
            Action::Continue => {
                dialog.refresh_preview(self.language);
                self.dialog = Some(dialog);
                return;
            }
            Action::Submit => {}
        }
        let scope = crate::operation::artifacts::Scope::new();
        let result = (|| -> crate::operation::Result<()> {
            let state = self
                .jobs
                .queue
                .as_ref()
                .map(|q| q.state.clone())
                .unwrap_or(crate::operation::durable::user_state()?);
            match dialog.submit(&state, &self.preferences)? {
                Submission::Update {
                    preview,
                    paths,
                    download,
                } => self
                    .adding
                    .apply_updates(&self.root, &state, preview, paths, download)?,
                Submission::GitHub(super::github::Action::Add(input)) => {
                    self.dialog = Some(super::dialog::Dialog::source(input))
                }
                Submission::GitHub(action) => self.adding.github(action)?,
                Submission::Source { input, options } => {
                    self.adding.source(&self.root, &state, input, options)?
                }
                Submission::CurseForge { selected, side } => {
                    self.adding.start(&self.root, &state, selected, side)?;
                }
                Submission::Prepared { request, label } => {
                    let queue = self.jobs.queue.as_mut().ok_or_else(|| {
                        crate::operation::Error::named(
                            crate::operation::ErrorCode::Busy,
                            "queue_unavailable",
                            "queue unavailable",
                        )
                    })?;
                    queue.enqueue(&label, request)?;
                    if let Some(artifacts) = &mut self.preparation_artifacts {
                        artifacts.release();
                    }
                }
                Submission::Resolve { index, keep } => {
                    let queue = self.jobs.queue.as_mut().ok_or_else(|| {
                        crate::operation::Error::named(
                            crate::operation::ErrorCode::Busy,
                            "queue_unavailable",
                            "queue unavailable",
                        )
                    })?;
                    queue.resolve(index, keep)?;
                }
                Submission::Edit(draft) => {
                    let queue = self.jobs.queue.as_mut().ok_or_else(|| {
                        crate::operation::Error::named(
                            crate::operation::ErrorCode::Busy,
                            "queue_unavailable",
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
        let mut submitted_artifacts = scope.finish();
        if result.is_ok() {
            submitted_artifacts.release();
            self.preparation_artifacts.take();
        }
        if let Err(error) = result {
            dialog.form.error = Some(self.language.error(&error));
            self.dialog = Some(dialog);
        }
    }

    fn switch_root(&mut self, root: PathBuf) -> crate::operation::Result<()> {
        if self.jobs.queue.as_ref().is_some_and(|q| q.pending()) {
            return Err(crate::operation::Error::named(
                crate::operation::ErrorCode::Busy,
                "queue_before_switch",
                "finish or cancel the current queue before switching packs",
            ));
        }
        let root = crate::operation::durable::canonical(&root)?;
        if root.exists() && !root.is_dir() {
            return Err(crate::operation::Error::named(
                crate::operation::ErrorCode::Invalid,
                "pack_not_directory",
                "not a directory",
            ));
        }
        self.preferences.remember(&root)?;
        self.root = root;
        self.catalog = super::catalog::Browser::default();
        self.adding = super::add::Workflow::default();
        self.pending = None;
        self.entries.clear();
        self.marked.clear();
        self.reset_cursor();
        self.jobs = super::jobs::Jobs::default();
        self.jobs.connect(self.root.clone());
        self.reload();
        self.page = if self.root.join("pack.toml").is_file() {
            0
        } else {
            2
        };
        if self.page == 2 {
            self.pack_selection
                .select(Some(super::pack::ACTIONS.len() - 1));
        }
        Ok(())
    }
}
