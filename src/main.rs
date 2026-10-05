mod app;
mod calendar;
mod changelog;
mod help;
mod input;
mod notes;
mod options;
mod search;
mod store;
mod tags;
#[cfg(test)]
mod test_util;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use chrono::Local;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use ratatui::crossterm::execute;

use crate::app::App;
use crate::options::Settings;
use crate::store::Store;

fn main() -> io::Result<()> {
    let today = Local::now().date_naive();
    let mut store = Store::load()?;
    store.roll_over(today)?;
    let mut app = App::new(store, today);
    app.settings_path = Settings::default_path();
    app.settings = Settings::load(app.settings_path.as_ref(), std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()));
    app.show_whats_new()?;
    let mut terminal = ratatui::init();
    // Have pasted text arrive in one piece rather than as typed keys. Old
    // Windows consoles can't, and then pasting works as typing, as before.
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = run(&mut terminal, &mut app);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
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
        app.tick(Instant::now());
        app.set_today(Local::now().date_naive())?;
    }
    Ok(())
}
