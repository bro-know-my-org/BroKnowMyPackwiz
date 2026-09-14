use super::{files::Entry, i18n::Language};
use crate::operation::{
    Error, ErrorCode, Event, Result, durable, edit,
    queue::{Queue, Request},
};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

#[derive(Default)]
pub struct Jobs {
    pub queue: Option<Queue>,
    pending: Option<Receiver<Result<Queue>>>,
    pub selected: usize,
    pub quit_prompt: bool,
    exit_when_idle: bool,
    list_area: Rect,
    list_offset: usize,
    log_area: Rect,
    log_scroll: usize,
}

impl Jobs {
    pub fn connect(&mut self, root: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = durable::user_state().and_then(|state| Queue::open(&root, &state));
            let _ = tx.send(result);
        });
    }

    pub fn poll(&mut self) -> Result<bool> {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    self.queue = Some(result?);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    return Err(Error::new(
                        ErrorCode::Interrupted,
                        "queue loader disconnected",
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.queue
            .as_mut()
            .map(Queue::poll)
            .transpose()
            .map(|v| v.unwrap_or(false))
    }

    pub fn pin(&mut self, root: &Path, entries: &[Entry]) -> Result<()> {
        let queue = self
            .queue
            .as_mut()
            .ok_or_else(|| Error::new(ErrorCode::Busy, "queue unavailable"))?;
        if entries.is_empty() {
            return Ok(());
        }
        let pin = !entries.iter().all(|e| e.metadata.pin);
        let mut drafts = Vec::new();
        for entry in entries {
            let (mut doc, expected) = edit::document(root, &entry.path)?;
            edit::set(&mut doc, &["pin"], toml_edit::Value::from(pin))?;
            drafts.push(edit::draft(&queue.state, &entry.path, expected, &doc)?);
        }
        queue.enqueue(if pin { "pin" } else { "unpin" }, Request::Edit(drafts))
    }

    pub fn key(&mut self, key: KeyCode) -> Result<()> {
        let Some(queue) = self.queue.as_mut() else {
            return Ok(());
        };
        match key {
            KeyCode::PageUp => self.log_scroll = self.log_scroll.saturating_add(10),
            KeyCode::PageDown => self.log_scroll = self.log_scroll.saturating_sub(10),
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(queue.tasks.len().saturating_sub(1))
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char(' ') => {
                if queue.paused {
                    queue.resume()?;
                } else {
                    queue.paused = true;
                }
            }
            KeyCode::Char('c') => queue.cancel(self.selected)?,
            KeyCode::Char('r') => queue.retry_recovery(self.selected)?,
            _ => {}
        }
        Ok(())
    }

    pub fn quit(&mut self) -> Result<bool> {
        if self.queue.as_ref().is_some_and(Queue::busy) {
            self.quit_prompt = true;
            Ok(false)
        } else {
            if let Some(queue) = &self.queue {
                queue.save()?;
            }
            Ok(true)
        }
    }

    pub fn quit_key(&mut self, key: KeyCode) -> Result<()> {
        match key {
            KeyCode::Esc => self.quit_prompt = false,
            KeyCode::Char('w') | KeyCode::Char('c') => {
                if let Some(queue) = &mut self.queue {
                    queue.paused = true;
                    if key == KeyCode::Char('c') && queue.busy() {
                        queue.cancel_current()?;
                    }
                    queue.save()?;
                }
                self.exit_when_idle = true;
                self.quit_prompt = false;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn should_exit(&self) -> bool {
        self.exit_when_idle
            && self
                .queue
                .as_ref()
                .is_none_or(|q| !q.busy() && !q.blocked())
    }

    pub fn mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let point = (mouse.column, mouse.row).into();
        if self.log_area.contains(point) {
            match mouse.kind {
                MouseEventKind::ScrollUp => self.log_scroll = self.log_scroll.saturating_add(3),
                MouseEventKind::ScrollDown => self.log_scroll = self.log_scroll.saturating_sub(3),
                _ => {}
            }
        } else if self.list_area.contains(point) {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let index = self.list_offset + (mouse.row - self.list_area.y) as usize;
                    if self.queue.as_ref().is_some_and(|q| index < q.tasks.len()) {
                        self.selected = index;
                    }
                }
                MouseEventKind::ScrollDown => {
                    let _ = self.key(KeyCode::Down);
                }
                MouseEventKind::ScrollUp => {
                    let _ = self.key(KeyCode::Up);
                }
                _ => {}
            }
        }
    }
}

pub fn draw(frame: &mut Frame, jobs: &mut Jobs, lang: Language, area: Rect) {
    jobs.list_area = Rect::default();
    jobs.log_area = Rect::default();
    let Some(queue) = &jobs.queue else {
        frame.render_widget(
            Paragraph::new(lang.text("loading")).block(Block::bordered()),
            area,
        );
        return;
    };
    let bands = Layout::vertical([
        Constraint::Percentage(45),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(area);
    let items: Vec<_> = queue
        .tasks
        .iter()
        .map(|task| {
            ListItem::new(format!(
                "{} · {}",
                lang.text(&task.label),
                lang.text(task.status.key())
            ))
        })
        .collect();
    let mut selection =
        ListState::default().with_selected((!items.is_empty()).then_some(jobs.selected));
    frame.render_stateful_widget(
        List::new(items).highlight_symbol("› ").block(
            Block::bordered().title(lang.text(if queue.paused { "paused" } else { "tasks" })),
        ),
        bands[0],
        &mut selection,
    );
    jobs.list_area = Block::bordered().inner(bands[0]);
    jobs.list_offset = selection.offset();
    jobs.log_area = Block::bordered().inner(bands[1]);
    let mut logs: Vec<_> = queue
        .logs
        .iter()
        .rev()
        .take(100)
        .map(|event| match event {
            Event::Phase(key) => lang.text(key).to_string(),
            Event::Progress {
                label,
                current,
                total,
            } => format!(
                "{label}: {current}/{}",
                total.map(|v| v.to_string()).unwrap_or_else(|| "?".into())
            ),
            Event::Log(text) => text.clone(),
        })
        .collect();
    logs.reverse();
    if let Some(error) = queue
        .tasks
        .get(jobs.selected)
        .and_then(|t| t.error.as_ref())
    {
        logs.push(error.to_string());
    }
    jobs.log_scroll = jobs.log_scroll.min(logs.len().saturating_sub(1));
    let start = logs
        .len()
        .saturating_sub(jobs.log_area.height as usize + jobs.log_scroll);
    frame.render_widget(
        Paragraph::new(logs[start..].join("\n"))
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(lang.text("logs"))),
        bands[1],
    );
    frame.render_widget(Paragraph::new(lang.text("task_keys")), bands[2]);
}
