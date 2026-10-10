mod app;
mod calendar;
mod changelog;
mod deadline;
mod cli;
mod help;
mod input;
mod markdown;
mod notes;
mod notes_files;
mod options;
mod search;
mod setup;
mod store;
mod tags;
#[cfg(test)]
mod test_util;
mod ui;
mod viewer;
mod workspaces;

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use chrono::Local;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen};

use crate::app::App;
use crate::options::Settings;
use crate::setup::Setup;
use crate::store::Store;
use crate::workspaces::Workspaces;

fn main() -> io::Result<()> {
    let settings_path = Settings::default_path();
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let mut settings = Settings::load(settings_path.as_ref(), no_color);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let todoro_file = std::env::var_os("TODORO_FILE").map(PathBuf::from);
    match cli::parse(&args, &settings, todoro_file.as_deref()) {
        cli::Run::App => {}
        cli::Run::Print(text) => {
            println!("{text}");
            return Ok(());
        }
        cli::Run::Fail(message, code) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    }

    let mut terminal = ratatui::init();
    // Have pasted text arrive in one piece rather than as typed keys. Old
    // Windows consoles can't, and then pasting works as typing, as before.
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = (|| loop {
        let Some((store, workspace)) = open(&mut terminal, &mut settings, settings_path.as_ref())? else {
            return Ok(());
        };
        let today = Local::now().date_naive();
        let mut app = App::new(store, today);
        app.settings = settings;
        app.settings_path = settings_path.clone();
        if let Some((workspaces, name)) = workspace {
            app.workspaces = Some(workspaces);
            app.workspace = Some(name);
        }
        app.store.roll_over(today)?;
        app.show_whats_new()?;
        run(&mut terminal, &mut app)?;
        if !app.restart {
            return Ok(());
        }
        // Reset: start again as if newly installed, from the settings file
        // the reset deleted.
        settings = Settings::load(settings_path.as_ref(), no_color);
    })();
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

/// The todos opened at startup, and the workspace they're in (`None` when
/// they come from `TODORO_FILE`).
type Opened = (Store, Option<(Workspaces, String)>);

/// Opens the todos to show: the file in `TODORO_FILE` if it's set (with no
/// workspaces), otherwise the workspace last used in the todoro folder. If
/// there's no folder yet, or it has no workspaces, the setup screen asks for
/// one first; quitting it gives `None`.
fn open(
    terminal: &mut ratatui::DefaultTerminal,
    settings: &mut Settings,
    settings_path: Option<&PathBuf>,
) -> io::Result<Option<Opened>> {
    if let Some(file) = std::env::var_os("TODORO_FILE") {
        return Ok(Some((Store::open(PathBuf::from(file))?, None)));
    }
    if let Some(dir) = settings.data_dir.clone()
        && let Some(name) = workspaces::pick(&dir, settings.workspace.as_deref())?
    {
        return Ok(Some(open_workspace(Workspaces::new(dir), name, settings, settings_path)?));
    }

    let legacy = workspaces::legacy_file().filter(|file| file.exists());
    let mut setup = Setup::new(settings.data_dir.clone(), legacy.clone());
    loop {
        terminal.draw(|frame| ui::draw_setup(frame, &setup))?;
        let action = match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => setup.handle_key(key),
            Event::Paste(text) => {
                setup.paste(&text);
                setup::Action::Stay
            }
            _ => setup::Action::Stay,
        };
        match action {
            setup::Action::Stay => {}
            setup::Action::Quit => return Ok(None),
            setup::Action::Done { dir, name } => match workspaces::set_up(&dir, &name, legacy.as_deref()) {
                Ok(name) => return Ok(Some(open_workspace(Workspaces::new(dir), name, settings, settings_path)?)),
                Err(error) => setup.error = Some(format!("Couldn't set that up: {error}")),
            },
        }
    }
}

/// Opens a workspace, remembering it (and its folder) for next time.
fn open_workspace(
    workspaces: Workspaces,
    name: String,
    settings: &mut Settings,
    settings_path: Option<&PathBuf>,
) -> io::Result<Opened> {
    let store = Store::open(workspaces.todos_path(&name))?;
    if settings.data_dir.as_ref() != Some(&workspaces.dir) || settings.workspace.as_ref() != Some(&name) {
        settings.data_dir = Some(workspaces.dir.clone());
        settings.workspace = Some(name.clone());
        if let Some(path) = settings_path {
            settings.save(path)?;
        }
    }
    Ok((store, Some((workspaces, name))))
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit && !app.restart {
        app.now = Local::now().time();
        terminal.draw(|frame| ui::draw(frame, app))?;
        // Wake up now and then even without a key, to notice midnight, or
        // sooner when something on screen (a copy's flash) is due to change.
        let wait = app.redraw_at().map_or(Duration::from_secs(30), |at| at.saturating_duration_since(Instant::now()));
        if event::poll(wait.min(Duration::from_secs(30)))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key)?,
                Event::Paste(text) => app.handle_paste(&text)?,
                _ => {}
            }
        }
        if let Some((command, path)) = app.start_external_edit()? {
            let result = edit_in(terminal, &command, &path);
            app.finish_external_edit(result)?;
        }
        app.tick(Instant::now());
        app.set_today(Local::now().date_naive())?;
    }
    Ok(())
}

/// Hands the terminal to the user's editor to edit `path`, and takes it back
/// once the editor closes.
fn edit_in(terminal: &mut ratatui::DefaultTerminal, command: &str, path: &Path) -> io::Result<ExitStatus> {
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    let status = editor_command(command, path).status();
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    terminal.clear()?;
    status
}

/// The command as typed with the file after it, run by the shell so quoted
/// arguments and variables like `$EDITOR` work.
#[cfg(not(windows))]
fn editor_command(command: &str, path: &Path) -> Command {
    let mut run = Command::new("sh");
    run.arg("-c").arg(format!("{command} \"$1\"")).arg("todoro").arg(path);
    run
}

/// The command as typed with the file after it, run by `cmd` so editors
/// installed as `.cmd` or `.bat` files (like VS Code's `code`) are found.
#[cfg(windows)]
fn editor_command(command: &str, path: &Path) -> Command {
    let mut run = Command::new("cmd");
    run.arg("/C").args(command.split_whitespace()).arg(path);
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn the_editor_command_gets_the_file_after_its_own_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a note's file.md");
        // Quoted arguments work, and a path with spaces and quotes stays whole.
        let status = editor_command("printf '%s|%s' 'one two'", &path)
            .stdout(std::fs::File::create(dir.path().join("out")).unwrap())
            .status()
            .unwrap();
        assert!(status.success());
        let out = std::fs::read_to_string(dir.path().join("out")).unwrap();
        assert_eq!(out, format!("one two|{}", path.display()));
    }
}
