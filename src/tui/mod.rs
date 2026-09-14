mod add;
mod app;
mod catalog;
mod dialog;
mod external;
mod files;
mod form;
mod i18n;
mod jobs;
mod preferences;
mod terminal;
#[cfg(test)]
mod tests;
mod view;

use crossterm::event;
use std::{path::PathBuf, time::Duration};

pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err("usage: bkmpw tui [pack-root]".into());
    }
    let root = args
        .first()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|err| err.to_string())?);
    let root = crate::operation::durable::canonical(&root).map_err(|err| err.to_string())?;
    let preferences = preferences::Preferences::load().map_err(|err| err.to_string())?;
    let (_session, mut screen) = terminal::Session::open().map_err(|err| err.to_string())?;
    let mut app = app::App::new(root);
    app.preferences = preferences;
    app.language = app
        .preferences
        .language
        .unwrap_or_else(i18n::Language::detect);
    app.preferences
        .remember(&app.root)
        .map_err(|err| err.to_string())?;
    app.persist_preferences = true;
    app.jobs.connect(app.root.clone());
    loop {
        if let Some(relative) = app.external.take() {
            let result = crate::operation::durable::user_state().and_then(|state| {
                external::External::prepare(&app.root, &relative, &state, &app.preferences)
            });
            match result {
                Ok(editor) => {
                    terminal::restore();
                    let result = editor.run();
                    terminal::activate().map_err(|e| e.to_string())?;
                    screen.clear().map_err(|e| e.to_string())?;
                    match result {
                        Ok(dialog) => app.dialog = Some(dialog),
                        Err(error) => app.error = Some(error.to_string()),
                    }
                }
                Err(error) => app.error = Some(error.to_string()),
            }
        }
        app.poll();
        if app.jobs.should_exit() {
            break;
        }
        screen
            .draw(|frame| view::draw(frame, &mut app))
            .map_err(|err| err.to_string())?;
        if event::poll(Duration::from_millis(100)).map_err(|err| err.to_string())?
            && app.event(event::read().map_err(|err| err.to_string())?)
        {
            break;
        }
    }
    Ok(())
}
