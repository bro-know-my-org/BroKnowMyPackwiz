use super::{app::App, files::Entry, i18n::Language, view};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend};

fn app() -> App {
    let mut app = App::new(std::env::temp_dir().join("bkmpw-nonexistent-test-pack"));
    app.page = 0;
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

fn click_action(app: &mut App, terminal: &mut Terminal<TestBackend>, code: KeyCode) -> bool {
    terminal.draw(|frame| view::draw(frame, app)).unwrap();
    let area = app
        .buttons
        .iter()
        .find(|(key, rect)| *key == code && rect.width > 0 && rect.height > 0)
        .unwrap_or_else(|| panic!("missing mouse action {code:?}"))
        .1;
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }))
}

#[test]
fn settings_wheel_and_click_track_the_visible_scrolled_row() {
    for language in [Language::En, Language::ZhCn] {
        let mut app = app();
        app.language = language;
        app.page = 4;
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        for _ in 1..super::dialog::SETTINGS.len() {
            terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
            app.event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: app.menu_area.x,
                row: app.menu_area.y,
                modifiers: KeyModifiers::NONE,
            }));
        }
        assert_eq!(app.settings_index, super::dialog::SETTINGS.len() - 1);
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let offset = app.settings_offset;
        assert!(offset > 0);
        app.event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: app.menu_area.x,
            row: app.menu_area.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.settings_index, offset);
        assert!(app.dialog.is_some());
    }
}

#[test]
fn file_search_filter_sort_and_details_have_mouse_actions() {
    for language in [Language::En, Language::ZhCn] {
        let mut app = app();
        app.language = language;
        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        click_action(&mut app, &mut terminal, KeyCode::Char('/'));
        assert!(app.editing);
        app.event(Event::Paste("中文".into()));
        click_action(&mut app, &mut terminal, KeyCode::Char('f'));
        assert!(!app.editing);
        assert_eq!(app.filter, "中文");
        assert_eq!(app.kind, 1);
        click_action(&mut app, &mut terminal, KeyCode::Char('s'));
        assert!(app.descending);
        click_action(&mut app, &mut terminal, KeyCode::Enter);
        assert!(app.detail);
        click_action(&mut app, &mut terminal, KeyCode::Enter);
        assert!(!app.detail);
        for (current, target) in [(0, 1), (1, 0)] {
            app.page = current;
            app.editing = current == 0;
            app.catalog.editing = current == 1;
            terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
            let tab = app.tabs[target];
            app.event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: tab.x,
                row: tab.y,
                modifiers: KeyModifiers::NONE,
            }));
            assert_eq!(app.page, target);
            assert!(!app.editing && !app.catalog.editing);
        }
    }
}

#[test]
fn mouse_can_open_close_help_and_errors_without_clicking_through() {
    for language in [Language::En, Language::ZhCn] {
        let mut app = app();
        app.language = language;
        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        assert!(!click_action(&mut app, &mut terminal, KeyCode::Char('?')));
        assert!(app.help);
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let tab = app.tabs[3];
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: tab.x,
            row: tab.y,
            modifiers: KeyModifiers::NONE,
        });
        app.event(click.clone());
        assert_eq!(app.page, 0);
        assert!(app.help);
        click_action(&mut app, &mut terminal, KeyCode::Esc);
        assert!(!app.help);
        app.error = Some("fixture error".into());
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        app.event(click);
        assert_eq!(app.page, 0);
        assert!(app.error.is_some());
        click_action(&mut app, &mut terminal, KeyCode::Esc);
        assert!(app.error.is_none());
        key(&mut app, KeyCode::Char('/'));
        click_action(&mut app, &mut terminal, KeyCode::Char('l'));
        assert!(!app.editing);
        assert!(app.filter.is_empty());
        assert_ne!(app.language, language);
        assert!(click_action(&mut app, &mut terminal, KeyCode::Char('q')));
    }
}

#[test]
fn quit_choices_and_recovery_retry_are_clickable_in_both_languages() {
    for language in [Language::En, Language::ZhCn] {
        for code in [KeyCode::Char('w'), KeyCode::Char('c'), KeyCode::Esc] {
            let mut app = app();
            app.language = language;
            app.jobs.quit_prompt = true;
            let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
            assert!(!click_action(&mut app, &mut terminal, code));
            assert!(!app.jobs.quit_prompt);
            assert_eq!(app.jobs.should_exit(), code != KeyCode::Esc);
        }
        let base = std::env::temp_dir().join(crate::operation::durable::unique_id());
        std::fs::create_dir_all(base.join("pack")).unwrap();
        let mut app = app();
        app.language = language;
        app.page = 3;
        app.jobs.queue = Some(
            crate::operation::queue::Queue::open(&base.join("pack"), &base.join("state")).unwrap(),
        );
        let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
        click_action(&mut app, &mut terminal, KeyCode::Char('r'));
        assert_eq!(app.error.as_deref(), Some(language.text("unknown_task")));
        drop(app);
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[test]
fn failed_queue_save_does_not_leak_a_new_form_draft() {
    use crate::operation::{durable, edit, queue::Queue};
    let base = std::env::temp_dir().join(durable::unique_id());
    let root = base.join("pack");
    std::fs::create_dir_all(root.join("mods")).unwrap();
    std::fs::write(root.join("pack.toml"), "name = 'Pack'").unwrap();
    std::fs::write(root.join("mods/a.pw.toml"), "name = 'A'").unwrap();
    let state = base.join("state");
    let mut app = App::new(root.clone());
    app.jobs.queue = Some(Queue::open(&root, &state).unwrap());
    std::fs::write(state.join("queues"), "block queue persistence").unwrap();
    let (document, expected) = edit::document(&root, "mods/a.pw.toml").unwrap();
    app.dialog = Some(super::dialog::Dialog::review(
        "mods/a.pw.toml".into(),
        expected,
        document,
        "Review".into(),
    ));
    key(&mut app, KeyCode::Enter);
    assert!(app.dialog.as_ref().unwrap().form.error.is_some());
    assert!(app.jobs.queue.as_ref().unwrap().tasks.is_empty());
    assert_eq!(std::fs::read_dir(state.join("drafts")).unwrap().count(), 0);
    drop(app);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn narrow_help_scrolls_to_the_end_and_keeps_shortcuts_modal() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
    for language in [Language::En, Language::ZhCn] {
        app.language = language;
        key(&mut app, KeyCode::Char('?'));
        assert!(app.help);
        assert_eq!(app.help_scroll, 0);
        key(&mut app, KeyCode::End);
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        let end = app.help_scroll;
        assert!(end > 8 && end < u16::MAX);
        key(&mut app, KeyCode::PageDown);
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        assert_eq!(app.help_scroll, end);
        key(&mut app, KeyCode::Home);
        assert_eq!(app.help_scroll, 0);
        app.event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 20,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.help_scroll, 3);
        assert_eq!(app.table.selected(), Some(0));
        assert!(!key(&mut app, KeyCode::Char('q')));
        assert!(!app.help);
        key(&mut app, KeyCode::Char('?'));
        assert_eq!(app.help_scroll, 0);
        key(&mut app, KeyCode::Esc);
        assert!(!app.help);
    }
}

#[test]
fn narrow_toolbar_keeps_all_file_actions_clickable() {
    let mut app = app();
    let mut terminal = Terminal::new(TestBackend::new(40, 20)).unwrap();
    for language in [Language::En, Language::ZhCn] {
        app.language = language;
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        for code in [
            KeyCode::Char('u'),
            KeyCode::Delete,
            KeyCode::Char('d'),
            KeyCode::Char('a'),
            KeyCode::Char('e'),
            KeyCode::Char('p'),
            KeyCode::Char('E'),
        ] {
            assert!(
                app.buttons
                    .iter()
                    .any(|(key, area)| *key == code && area.width > 0)
            );
        }
    }
}

#[test]
fn new_directory_starts_at_initialization_and_keeps_open_directory_available() {
    let root = std::env::temp_dir().join(crate::operation::durable::unique_id());
    let mut app = App::new(root);
    assert_eq!(app.page, 2);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.dialog.as_ref().unwrap().form.title, "init");
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.page, 4);
}

#[test]
fn pack_menu_scroll_click_and_confirmation_enqueue_typed_command() {
    use crate::operation::{
        command::Kind,
        durable,
        queue::{Queue, Request},
    };
    let base = std::env::temp_dir().join(durable::unique_id());
    std::fs::create_dir_all(base.join("pack")).unwrap();
    let mut app = app();
    app.root = base.join("pack");
    app.jobs.queue = Some(Queue::open(&app.root, &base.join("state")).unwrap());
    app.page = 2;
    app.error = None;
    let mut terminal = Terminal::new(TestBackend::new(80, 15)).unwrap();
    for _ in 0..16 {
        key(&mut app, KeyCode::Down);
    }
    terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
    assert!(app.pack_selection.offset() > 0);
    let index = app.pack_selection.offset();
    let area = app.menu_area;
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(app.pack_selection.selected(), Some(index));
    assert!(app.dialog.is_some());
    assert!(app.jobs.queue.as_ref().unwrap().tasks.is_empty());
    key(&mut app, KeyCode::Esc);
    app.pack_selection.select(Some(0));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    assert!(
        matches!(app.jobs.queue.as_ref().unwrap().tasks[0].request, Request::Command(ref request) if request.kind == Kind::Inspect)
    );
    drop(app);
    std::fs::remove_dir_all(base).unwrap();
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
    let guard = Guard::capture(&root, &Control::default()).unwrap();
    let preview = || crate::catalog::prepare::Preview {
        rows: Vec::new(),
        drafts: vec![
            edit::draft(
                &state,
                "mods/new.pw.toml",
                None,
                &"name = \"New\"".parse().unwrap(),
            )
            .unwrap(),
        ],
        guard: guard.clone(),
        downloads: Vec::new(),
        download: false,
    };
    let mut app = App::new(root.clone());
    app.jobs.queue = Some(Queue::open(&root, &state).unwrap());
    app.dialog = Some(super::dialog::Dialog::prepared(preview(), Language::ZhCn));
    key(&mut app, KeyCode::Esc);
    assert!(app.jobs.queue.as_ref().unwrap().tasks.is_empty());
    app.dialog = Some(super::dialog::Dialog::prepared(preview(), Language::En));
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.jobs.queue.as_ref().unwrap().tasks.len(), 1);
    let mut with_download = preview();
    with_download.download = true;
    app.dialog = Some(super::dialog::Dialog::prepared(with_download, Language::En));
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.jobs.queue.as_ref().unwrap().tasks[1].request,
        crate::operation::queue::Request::Download { .. }
    ));
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

#[test]
fn error_popup_scrolls_unicode_and_clamps_after_resize_or_replacement() {
    for language in [Language::En, Language::ZhCn] {
        let mut app = app();
        app.language = language;
        app.error = Some(format!("{}\nFINAL_MARKER", "中文路径 abc ".repeat(100)));
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        key(&mut app, KeyCode::End);
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        assert!(app.popup_scroll > 0);
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("FINAL_MARKER"));
        let end = app.popup_scroll;
        app.event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 3,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.popup_scroll, end.saturating_sub(3));
        key(&mut app, KeyCode::Home);
        assert_eq!(app.popup_scroll, 0);
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.popup_scroll, 8);
        key(&mut app, KeyCode::End);
        let mut large = Terminal::new(TestBackend::new(200, 100)).unwrap();
        large.draw(|frame| view::draw(frame, &mut app)).unwrap();
        assert_eq!(app.popup_scroll, 0);
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        key(&mut app, KeyCode::End);
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        app.error = Some("Replacement".into());
        terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
        assert_eq!(app.popup_scroll, 0);
        click_action(&mut app, &mut terminal, KeyCode::Esc);
        assert!(app.error.is_none());
        assert!(app.popup_text.is_empty());
    }
}

#[test]
fn error_popup_consumes_background_shortcuts_and_paste_in_both_search_fields() {
    for page in [0, 1] {
        let mut app = app();
        app.page = page;
        app.editing = page == 0;
        app.catalog.editing = page == 1;
        app.error = Some("Failure".into());
        let language = app.language;
        for code in [
            KeyCode::Tab,
            KeyCode::Char('l'),
            KeyCode::Delete,
            KeyCode::Char('r'),
        ] {
            assert!(!key(&mut app, code));
        }
        app.event(Event::Paste("unexpected".into()));
        assert_eq!(app.page, page);
        assert_eq!(app.language, language);
        assert!(app.filter.is_empty());
        assert!(app.catalog.query.is_empty());
        assert!(app.dialog.is_none());
        assert!(app.error.is_some());
        key(&mut app, KeyCode::Enter);
        assert!(app.error.is_none());
        assert_eq!(app.popup_scroll, 0);
    }
}

#[test]
fn file_details_scroll_without_changing_selection_and_reset_on_new_entry() {
    for language in [Language::En, Language::ZhCn] {
        for width in [80, 120] {
            let mut app = app();
            app.language = language;
            app.detail = true;
            app.entries[1].metadata.filename = Some("中文长文件名".repeat(80));
            let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
            terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
            key(&mut app, KeyCode::End);
            terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
            assert!(app.detail_view.scroll > 0);
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(
                text.replace(' ', "")
                    .contains(&language.text("preserve").replace(' ', ""))
            );
            let selected = app.table.selected();
            let scroll = app.detail_view.scroll;
            app.event(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: app.detail_view.area.x,
                row: app.detail_view.area.y,
                modifiers: KeyModifiers::NONE,
            }));
            assert_eq!(app.detail_view.scroll, scroll.saturating_sub(3));
            assert_eq!(app.table.selected(), selected);
            key(&mut app, KeyCode::Down);
            terminal.draw(|frame| view::draw(frame, &mut app)).unwrap();
            assert_ne!(app.table.selected(), selected);
            assert_eq!(app.detail_view.scroll, 0);
        }
    }
}
