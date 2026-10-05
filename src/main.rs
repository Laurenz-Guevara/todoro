mod app;
mod calendar;
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
use std::time::Duration;

use chrono::Local;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

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
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        // Wake up now and then even without a key, to notice midnight.
        if event::poll(Duration::from_secs(30))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key)?;
        }
        app.set_today(Local::now().date_naive())?;
    }
    Ok(())
}
