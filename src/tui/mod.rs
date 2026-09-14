mod i18n;
mod terminal;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::widgets::{Block, Paragraph};

pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err("usage: bkmpw tui [pack-root]".into());
    }
    let (_session, mut screen) = terminal::Session::open().map_err(|err| err.to_string())?;
    let mut language = i18n::Language::detect();
    loop {
        screen
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(language.text("keys")).block(Block::bordered().title("bkmpw")),
                    frame.area(),
                );
            })
            .map_err(|err| err.to_string())?;
        if let Event::Key(key) = event::read().map_err(|err| err.to_string())? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('l') => language.toggle(),
                _ => {}
            }
        }
    }
    Ok(())
}
