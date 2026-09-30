use super::{Result, upgrade};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    widgets::{Block, Borders, List, ListState, Paragraph, Wrap},
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

struct Screen(Terminal<CrosstermBackend<io::Stdout>>);
impl Screen {
    fn new() -> Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(
                "NeoForge TUI requires an interactive terminal; use CLI plan/upgrade in scripts"
                    .into(),
            );
        }
        terminal::enable_raw_mode().map_err(|e| e.to_string())?;
        let result = (|| {
            execute!(io::stdout(), EnterAlternateScreen).map_err(|e| e.to_string())?;
            Terminal::new(CrosstermBackend::new(io::stdout()))
                .map(Self)
                .map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
        result
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(self.0.backend_mut(), LeaveAlternateScreen);
        let _ = self.0.show_cursor();
    }
}
fn choose(title: &str, detail: &str, entries: &[String]) -> Result<Option<usize>> {
    if entries.is_empty() {
        return Err("no stable NeoForge candidates for this Minecraft version".into());
    }
    let mut screen = Screen::new()?;
    let mut selected = 0;
    let mut scroll: u16 = 0;
    loop {
        screen.0.draw(|frame| {
            let areas = Layout::vertical([Constraint::Percentage(65), Constraint::Percentage(35)]).split(frame.area());
            frame.render_widget(Paragraph::new(detail).block(Block::default().title(title).borders(Borders::ALL)).wrap(Wrap { trim: false }).scroll((scroll, 0)), areas[0]);
            let mut state = ListState::default().with_selected(Some(selected));
            frame.render_stateful_widget(List::new(entries.iter().map(String::as_str)).block(Block::default().title("↑/↓ Select · Enter Confirm · Esc Cancel · PgUp/PgDn Read report").borders(Borders::ALL)).highlight_symbol("→ "), areas[1], &mut state);
        }).map_err(|e| e.to_string())?;
        if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => selected = (selected + 1).min(entries.len() - 1),
                KeyCode::PageDown => scroll = scroll.saturating_add(10),
                KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                KeyCode::Enter => return Ok(Some(selected)),
                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}
fn instance_path(initial: &str) -> Result<Option<PathBuf>> {
    let mut screen = Screen::new()?;
    let mut path = initial.to_owned();
    loop {
        screen.0.draw(|frame| { frame.render_widget(Paragraph::new(format!("Select a stopped standard NeoForge server or a Prism instance directory.\nPrism: choose the directory containing mmc-pack.json, not minecraft/.\nClose Prism Launcher before upgrading.\n\n{path}\n\nType path · Backspace Edit · Enter Select · Esc Cancel")).block(Block::default().title("NeoForge in-place upgrade · Instance").borders(Borders::ALL)).wrap(Wrap { trim: false }), frame.area()); }).map_err(|e| e.to_string())?;
        if let Event::Key(key) = event::read().map_err(|e| e.to_string())? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Backspace => {
                    path.pop();
                }
                KeyCode::Enter => {
                    return PathBuf::from(&path)
                        .canonicalize()
                        .map(Some)
                        .map_err(|e| e.to_string());
                }
                KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                KeyCode::Char(c) => path.push(c),
                _ => {}
            }
        }
    }
}
pub fn run(args: &[String]) -> Result<()> {
    let usage = "usage: bkmpw neoforge tui [instance] [--java PATH] [--script RELATIVE_PATH] [--sync-source ROOT] [--installer JAR]";
    if args.iter().any(|a| a == "--help") {
        println!("{usage}");
        return Ok(());
    }
    let has_root = args.first().is_some_and(|a| !a.starts_with("--"));
    let tail = if has_root { &args[1..] } else { args };
    if !tail.len().is_multiple_of(2)
        || tail.chunks(2).any(|p| {
            !matches!(
                p[0].as_str(),
                "--java" | "--script" | "--sync-source" | "--installer"
            )
        })
    {
        return Err(usage.into());
    }
    let Some(root) = instance_path(if has_root { args[0].as_str() } else { "" })? else {
        return Ok(());
    };
    let mut parsed = vec!["plan".into(), root.to_string_lossy().into_owned()];
    parsed.extend_from_slice(tail);
    let (_, mut options, _, _) = super::cli::parse(&parsed)?;
    let _guards = upgrade::guards(&root, options.external.as_deref())?;
    let instance = super::detect(&root, &options.scripts)?;
    println!(
        "Fetching stable candidates for Minecraft {}…",
        instance.minecraft
    );
    let index = super::get_json(&format!("{}/index.json", super::META))?;
    let candidates = super::candidates(&index, &instance.minecraft)?;
    let Some(selected) = choose(
        "Choose target NeoForge",
        &format!(
            "Instance: {}\nKind: {:?}\nMinecraft: {}\nCurrent NeoForge: {}\nOnly stable targets for this exact Minecraft version are listed.",
            root.display(),
            instance.kind,
            instance.minecraft,
            instance.version
        ),
        &candidates,
    )?
    else {
        return Ok(());
    };
    options.target = candidates[selected].clone();
    println!("Verifying official installer and checking installed mods…");
    let plan = upgrade::prepare(&options)?;
    let detail = serde_json::to_string_pretty(&plan.summary()).map_err(|e| e.to_string())?;
    if !plan.report.blocked.is_empty() {
        choose(
            "Incompatible mods · upgrade blocked",
            &detail,
            &["Cancel".into()],
        )?;
        return Err("incompatible mods block this upgrade".into());
    }
    let choices = vec![
        "Cancel".into(),
        if plan.report.unknown.is_empty() {
            "Upgrade; keep backups".into()
        } else {
            "Accept listed unknown results and upgrade; keep backups".into()
        },
    ];
    if choose(
        "Compatibility precheck · Read report before continuing",
        &detail,
        &choices,
    )? != Some(1)
    {
        return Ok(());
    }
    options.accept_unknown = !plan.report.unknown.is_empty();
    println!("Installing and switching NeoForge; originals will be backed up…");
    println!(
        "{}",
        serde_json::to_string_pretty(&upgrade::execute(&plan, &options)?)
            .map_err(|e| e.to_string())?
    );
    Ok(())
}
