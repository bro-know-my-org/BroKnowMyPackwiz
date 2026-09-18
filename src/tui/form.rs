use super::i18n::Language;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Clear, Paragraph},
};

#[derive(Clone)]
pub enum Kind {
    Text,
    Secret,
    Number,
    Bool,
    Choice(Vec<String>),
}
#[derive(Clone)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub value: String,
    pub initial: String,
    pub kind: Kind,
    cursor: usize,
}
impl Field {
    pub fn new(key: &str, label: &str, value: impl Into<String>, kind: Kind) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self {
            key: key.into(),
            label: label.into(),
            initial: value.clone(),
            value,
            kind,
            cursor,
        }
    }
    fn insert(&mut self, value: &str) {
        let value: String = value.chars().filter(|c| !c.is_control()).collect();
        self.value.insert_str(self.cursor, &value);
        self.cursor += value.len();
    }
    fn previous(&self) -> usize {
        self.value[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
    fn next(&self) -> usize {
        self.cursor
            + self.value[self.cursor..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(0)
    }
    fn cycle(&mut self, reverse: bool) {
        match &self.kind {
            Kind::Bool => self.value = (self.value != "true").to_string(),
            Kind::Choice(options) if !options.is_empty() => {
                let index = options.iter().position(|v| v == &self.value).unwrap_or(0);
                self.value = options
                    [(index + if reverse { options.len() - 1 } else { 1 }) % options.len()]
                .clone();
            }
            _ => {}
        }
        self.cursor = self.value.len();
    }
}

pub struct Form {
    pub checklist: bool,
    checklist_scroll: u16,
    pub title: String,
    pub fields: Vec<Field>,
    pub focus: usize,
    pub error: Option<String>,
    pub preview: Option<String>,
    preview_scroll: u16,
    preview_area: Rect,
    preview_text: String,
    areas: Vec<(usize, Rect)>,
}
#[derive(PartialEq, Eq, Debug)]
pub enum Action {
    Continue,
    Submit,
    Cancel,
}

impl Form {
    pub fn new(title: &str, fields: Vec<Field>) -> Self {
        Self {
            checklist: false,
            checklist_scroll: 0,
            title: title.into(),
            fields,
            focus: 0,
            error: None,
            preview: None,
            preview_scroll: 0,
            preview_area: Rect::default(),
            preview_text: String::new(),
            areas: Vec::new(),
        }
    }
    pub fn value(&self, key: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.as_str())
            .unwrap_or("")
    }
    pub fn event(&mut self, event: Event) -> Action {
        let previous_focus = self.focus;
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Left if self.checklist => {
                    self.checklist_scroll = self.checklist_scroll.saturating_sub(8);
                }
                KeyCode::Right if self.checklist => {
                    self.checklist_scroll = self.checklist_scroll.saturating_add(8);
                }
                KeyCode::PageDown if self.preview.is_some() => {
                    self.preview_scroll = self.preview_scroll.saturating_add(10)
                }
                KeyCode::PageUp if self.preview.is_some() => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(10)
                }
                KeyCode::Home if self.preview.is_some() => self.preview_scroll = 0,
                KeyCode::End if self.preview.is_some() => self.preview_scroll = u16::MAX,
                KeyCode::Esc => return Action::Cancel,
                KeyCode::Tab | KeyCode::Down => {
                    self.focus = (self.focus + 1) % (self.fields.len() + 2)
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.focus = (self.focus + self.fields.len() + 1) % (self.fields.len() + 2)
                }
                KeyCode::Enter if self.focus == self.fields.len() => return Action::Submit,
                KeyCode::Enter if self.focus > self.fields.len() => return Action::Cancel,
                KeyCode::Enter if self.checklist => return Action::Submit,
                KeyCode::Char('a')
                    if self.checklist
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    let selected = self
                        .fields
                        .iter()
                        .filter(|f| f.key != "download")
                        .all(|f| f.value == "true");
                    for field in self.fields.iter_mut().filter(|f| f.key != "download") {
                        field.value = (!selected).to_string();
                    }
                }
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Action::Submit;
                }
                KeyCode::Enter => self.focus = (self.focus + 1).min(self.fields.len()),
                code if self.focus < self.fields.len() => {
                    let field = &mut self.fields[self.focus];
                    if matches!(field.kind, Kind::Bool | Kind::Choice(_)) {
                        if matches!(code, KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')) {
                            field.cycle(code == KeyCode::Left);
                        }
                    } else {
                        match code {
                            KeyCode::Home => field.cursor = 0,
                            KeyCode::End => field.cursor = field.value.len(),
                            KeyCode::Left => field.cursor = field.previous(),
                            KeyCode::Right => field.cursor = field.next(),
                            KeyCode::Backspace => {
                                let previous = field.previous();
                                field.value.drain(previous..field.cursor);
                                field.cursor = previous;
                            }
                            KeyCode::Delete => {
                                let next = field.next();
                                field.value.drain(field.cursor..next);
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                field.value.clear();
                                field.cursor = 0;
                            }
                            KeyCode::Char(c)
                                if !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                field.insert(&c.to_string())
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Event::Paste(text) if self.focus < self.fields.len() => {
                let field = &mut self.fields[self.focus];
                if matches!(field.kind, Kind::Text | Kind::Secret | Kind::Number) {
                    field.insert(&text);
                }
            }
            Event::Mouse(mouse) => {
                if self.preview.is_some()
                    && self.preview_area.contains((mouse.column, mouse.row).into())
                {
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            self.preview_scroll = self.preview_scroll.saturating_add(3);
                            return Action::Continue;
                        }
                        MouseEventKind::ScrollUp => {
                            self.preview_scroll = self.preview_scroll.saturating_sub(3);
                            return Action::Continue;
                        }
                        _ => {}
                    }
                }
                if matches!(
                    mouse.kind,
                    MouseEventKind::Down(MouseButton::Left | MouseButton::Right)
                ) {
                    let reverse = mouse.kind == MouseEventKind::Down(MouseButton::Right);
                    if let Some((index, _)) = self
                        .areas
                        .iter()
                        .find(|(_, r)| r.contains((mouse.column, mouse.row).into()))
                    {
                        self.focus = *index;
                        if reverse && *index >= self.fields.len() {
                            return Action::Continue;
                        }
                        if *index == self.fields.len() {
                            return Action::Submit;
                        }
                        if *index > self.fields.len() {
                            return Action::Cancel;
                        }
                        if matches!(self.fields[*index].kind, Kind::Bool | Kind::Choice(_)) {
                            self.fields[*index].cycle(reverse);
                        }
                    }
                } else if mouse.kind == MouseEventKind::ScrollDown {
                    self.focus = (self.focus + 1).min(self.fields.len() + 1);
                } else if mouse.kind == MouseEventKind::ScrollUp {
                    self.focus = self.focus.saturating_sub(1);
                }
            }
            _ => {}
        }
        if self.focus != previous_focus {
            self.checklist_scroll = 0;
        }
        Action::Continue
    }

    pub fn draw(&mut self, frame: &mut Frame, lang: Language) {
        self.preview_area = Rect::default();
        if self.preview_text != self.preview.as_deref().unwrap_or_default() {
            self.preview_text = self.preview.clone().unwrap_or_default();
            self.preview_scroll = 0;
        }
        let area = frame.area();
        let rect = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        frame.render_widget(Clear, rect);
        let block = Block::bordered().title(lang.text(&self.title));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        self.areas.clear();
        if inner.height < 7 {
            frame.render_widget(Paragraph::new(lang.text("small")), inner);
            return;
        }
        let field_height = if self.checklist && self.preview.is_some() {
            ((inner.height.saturating_sub(4) / 2 / 3).max(1)) * 3
        } else {
            inner.height.saturating_sub(4)
        };
        let count = (field_height / 3).max(1) as usize;
        let offset = self
            .focus
            .min(self.fields.len().saturating_sub(1))
            .saturating_sub(count - 1);
        for (row, i) in (offset..self.fields.len()).take(count).enumerate() {
            let field = &self.fields[i];
            let area = Rect::new(inner.x, inner.y + row as u16 * 3, inner.width, 3);
            if self.checklist {
                let text = format!(
                    "[{}] {}",
                    if field.value == "true" { "x" } else { " " },
                    lang.text(&field.label)
                );
                let overflow = text
                    .lines()
                    .map(unicode_width::UnicodeWidthStr::width)
                    .max()
                    .unwrap_or(0)
                    .saturating_sub(area.width as usize)
                    .min(u16::MAX as usize) as u16;
                if self.focus == i {
                    self.checklist_scroll = self.checklist_scroll.min(overflow);
                }
                frame.render_widget(
                    Paragraph::new(text)
                        .scroll((0, self.checklist_scroll.min(overflow)))
                        .style(if self.focus == i {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        }),
                    area,
                );
                self.areas.push((i, area));
                continue;
            }
            let text = match field.kind {
                Kind::Secret => "•".repeat(field.value.chars().count()),
                Kind::Bool => lang
                    .text(if field.value == "true" { "yes" } else { "no" })
                    .to_string(),
                _ => field.value.clone(),
            };
            let marker = match field.kind {
                Kind::Secret => field.value[..field.cursor].chars().count() * "•".len(),
                Kind::Bool | Kind::Choice(_) => text.len(),
                _ => field.cursor,
            };
            let scroll = unicode_width::UnicodeWidthStr::width(&text[..marker])
                .saturating_sub(area.width.saturating_sub(4) as usize)
                as u16;
            let mut display = text;
            if self.focus == i {
                display.insert_str(marker, "▏");
            }
            frame.render_widget(
                Paragraph::new(display)
                    .block(Block::bordered().title(lang.text(&field.label)))
                    .scroll((0, scroll))
                    .style(if self.focus == i {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    }),
                area,
            );
            self.areas.push((i, area));
        }
        let footer = Rect::new(inner.x, inner.bottom().saturating_sub(4), inner.width, 3);
        if let Some(preview) = &self.preview {
            let offset = if self.checklist { field_height } else { 0 };
            let area = Rect::new(
                inner.x,
                inner.y + offset,
                inner.width,
                inner.height.saturating_sub(4 + offset),
            );
            self.preview_area = area;
            super::view::scroll_text(frame, preview, area, &mut self.preview_scroll);
        }
        for (i, area) in Layout::horizontal([Constraint::Percentage(50); 2])
            .split(footer)
            .iter()
            .enumerate()
        {
            let index = self.fields.len() + i;
            frame.render_widget(
                Paragraph::new(lang.text(if i == 0 { "save" } else { "cancel" }))
                    .block(Block::bordered())
                    .style(if self.focus == index {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    }),
                *area,
            );
            self.areas.push((index, *area));
        }
        let hint = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
        frame.render_widget(
            Paragraph::new(
                self.error
                    .as_deref()
                    .unwrap_or(lang.text(if self.checklist {
                        "update_selection_keys"
                    } else if self.preview.is_some() {
                        "preview_keys"
                    } else {
                        "form_keys"
                    })),
            ),
            hint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    fn key(form: &mut Form, code: KeyCode) -> Action {
        form.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }
    #[test]
    fn preview_scroll_keeps_tail_visible_and_mouse_preserves_focus() {
        use crossterm::event::MouseEvent;
        use ratatui::{Terminal, backend::TestBackend};
        for lang in [Language::ZhCn, Language::En] {
            let mut form = Form::new("test", Vec::new());
            form.preview = Some(format!("{}\nTAIL", "中文预览内容".repeat(100)));
            let mut screen = Terminal::new(TestBackend::new(40, 12)).unwrap();
            screen.draw(|frame| form.draw(frame, lang)).unwrap();
            let area = form.preview_area;
            let focus = form.focus;
            form.event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }));
            assert_eq!(form.preview_scroll, 3);
            assert_eq!(form.focus, focus);
            key(&mut form, KeyCode::End);
            screen.draw(|frame| form.draw(frame, lang)).unwrap();
            assert!(form.preview_scroll > 0 && form.preview_scroll < u16::MAX);
            let text = |screen: &Terminal<TestBackend>| {
                screen
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
            };
            assert!(text(&screen).contains("TAIL"));
            let bottom = form.preview_scroll;
            key(&mut form, KeyCode::PageDown);
            screen.draw(|frame| form.draw(frame, lang)).unwrap();
            assert_eq!(form.preview_scroll, bottom);
            screen.backend_mut().resize(60, 20);
            screen.draw(|frame| form.draw(frame, lang)).unwrap();
            assert!(text(&screen).contains("TAIL"));
            form.preview = Some("replacement".into());
            screen.draw(|frame| form.draw(frame, lang)).unwrap();
            assert_eq!(form.preview_scroll, 0);
            assert!(text(&screen).contains("replacement"));
            assert_eq!(key(&mut form, KeyCode::Enter), Action::Submit);
        }
    }
    #[test]
    fn mouse_choices_cycle_both_ways_without_right_click_submitting() {
        let mut form = Form::new(
            "test",
            vec![Field::new(
                "choice",
                "name",
                "first",
                Kind::Choice(vec!["first".into(), "second".into(), "third".into()]),
            )],
        );
        let mut screen =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 15)).unwrap();
        screen
            .draw(|frame| form.draw(frame, Language::ZhCn))
            .unwrap();
        let click = |form: &mut Form, index: usize, button| {
            let area = form.areas.iter().find(|(i, _)| *i == index).unwrap().1;
            form.event(Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(button),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            }))
        };
        assert_eq!(click(&mut form, 0, MouseButton::Right), Action::Continue);
        assert_eq!(form.value("choice"), "third");
        assert_eq!(click(&mut form, 0, MouseButton::Left), Action::Continue);
        assert_eq!(form.value("choice"), "first");
        for index in [1, 2] {
            assert_eq!(
                click(&mut form, index, MouseButton::Right),
                Action::Continue
            );
        }
        assert_eq!(click(&mut form, 1, MouseButton::Left), Action::Submit);
        assert_eq!(click(&mut form, 2, MouseButton::Left), Action::Cancel);
    }

    #[test]
    fn unicode_cursor_paste_and_form_navigation_are_safe() {
        let mut form = Form::new("test", vec![Field::new("x", "name", "中文", Kind::Text)]);
        key(&mut form, KeyCode::Left);
        key(&mut form, KeyCode::Backspace);
        assert_eq!(form.value("x"), "文");
        form.event(Event::Paste("你好\n".into()));
        assert_eq!(form.value("x"), "你好文");
        key(&mut form, KeyCode::Delete);
        assert_eq!(form.value("x"), "你好");
        assert_eq!(key(&mut form, KeyCode::Char('q')), Action::Continue);
        key(&mut form, KeyCode::Tab);
        assert_eq!(key(&mut form, KeyCode::Enter), Action::Submit);
    }

    #[test]
    fn translated_booleans_and_secrets_render_at_small_sizes() {
        let mut form = Form::new(
            "settings",
            vec![
                Field::new("enabled", "pin", "true", Kind::Bool),
                Field::new("key", "api_key", "private-key-value", Kind::Secret),
            ],
        );
        for (width, height) in [(1, 1), (40, 10), (80, 24)] {
            for language in [Language::ZhCn, Language::En] {
                let mut screen =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                        .unwrap();
                screen.draw(|frame| form.draw(frame, language)).unwrap();
                let text: String = screen
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(!text.contains("private-key-value"));
            }
        }
    }
}
