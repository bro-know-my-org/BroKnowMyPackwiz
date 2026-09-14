mod app;
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
    let (_session, mut screen) = terminal::Session::open().map_err(|err| err.to_string())?;
    let mut app = app::App::new(root);
    app.jobs.connect(app.root.clone());
    loop {
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
