use super::{app::App, files::Entry, i18n::Language, view};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};

fn app() -> App {
    let mut app = App::new(std::env::temp_dir().join("bkmpw-nonexistent-test-pack"));
    app.entries = ["中文包", "Apple", "Zebra"]
        .iter()
        .map(|name| Entry {
            path: format!("mods/{name}.pw.toml"),
            name: name.to_string(),
            source: "URL".into(),
            side: "both".into(),
            present: false,
            metadata: crate::metadata::ModMetadata::parse(""),
        })
        .collect();
    app.table.select(Some(0));
    app
}

fn key(app: &mut App, code: KeyCode) -> bool {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

#[test]
fn search_accepts_unicode_and_does_not_run_shortcuts() {
    let mut app = app();
    key(&mut app, KeyCode::Char('/'));
    app.event(Event::Paste("中文\n".into()));
    assert_eq!(app.filter, "中文");
    assert_eq!(app.visible().len(), 1);
    assert!(!key(&mut app, KeyCode::Char('q')));
    assert_eq!(app.filter, "中文q");
    key(&mut app, KeyCode::Backspace);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.filter, "中");
    key(&mut app, KeyCode::Esc);
    assert!(!app.editing);
    assert!(key(&mut app, KeyCode::Char('q')));
}

#[test]
fn catalog_query_keeps_global_shortcuts_inside_the_input() {
    let mut app = app();
    app.page = 1;
    key(&mut app, KeyCode::Char('/'));
    assert!(!key(&mut app, KeyCode::Char('q')));
    key(&mut app, KeyCode::Char('l'));
    app.event(Event::Paste("中文".into()));
    assert_eq!(app.catalog.query, "ql中文");
    assert_eq!(app.page, 1);
    key(&mut app, KeyCode::Esc);
    assert!(!app.catalog.editing);
}

#[test]
fn dependency_confirmation_is_required_before_enqueueing() {
    use crate::operation::{Control, durable, edit, preview::Guard, queue::Queue};
    use std::fs;
    let base = std::env::temp_dir().join(durable::unique_id());
    fs::create_dir_all(base.join("pack/mods")).unwrap();
    let base = fs::canonicalize(base).unwrap();
    let root = base.join("pack");
    fs::write(root.join("pack.toml"), "name = \"Test\"\n").unwrap();
    let state = base.join("state");
    let draft = edit::draft(
        &state,
        "mods/new.pw.toml",
        None,
        &"name = \"New\"".parse().unwrap(),
    )
    .unwrap();
    let guard = Guard::capture(&root, &Control::default()).unwrap();
    let preview = || crate::catalog::prepare::Preview {
        rows: Vec::new(),
        drafts: vec![draft.clone()],
        guard: guard.clone(),
    };
    let mut app = App::new(root.clone());
    app.jobs.queue = Some(Queue::open(&root, &state).unwrap());
    app.dialog = Some(super::dialog::Dialog::prepared(preview(), Language::ZhCn));
    key(&mut app, KeyCode::Esc);
    assert!(app.jobs.queue.as_ref().unwrap().tasks.is_empty());
    app.dialog = Some(super::dialog::Dialog::prepared(preview(), Language::En));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.jobs.queue.as_ref().unwrap().tasks.len(), 1);
    assert!(!root.join("mods/new.pw.toml").exists());
    drop(app);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn marks_follow_identity_across_sort_and_filter() {
    let mut app = app();
    key(&mut app, KeyCode::Char(' '));
    assert!(app.marked.contains("mods/Apple.pw.toml"));
    key(&mut app, KeyCode::Char('s'));
    assert!(app.marked.contains("mods/Apple.pw.toml"));
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('f'));
    assert!(app.visible().is_empty());
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.marked.len(), 1);
    assert!(app.table.selected().is_none());
}

#[test]
fn rendering_handles_languages_small_windows_and_mouse_row() {
    let mut app = app();
    for lang in [Language::En, Language::ZhCn] {
        app.language = lang;
        for (width, height) in [(1, 1), (39, 9), (40, 10), (80, 24), (120, 40)] {
            let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
            screen.draw(|frame| view::draw(frame, &mut app)).unwrap();
            app.detail = true;
            screen.draw(|frame| view::draw(frame, &mut app)).unwrap();
            app.detail = false;
        }
    }
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: app.table_area.x,
        row: app.table_area.y + 1,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(app.table.selected(), Some(1));
    assert!(app.marked.contains("mods/Zebra.pw.toml"));
}
