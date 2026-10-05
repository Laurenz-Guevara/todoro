mod app;
mod help;
mod notes;
mod store;
#[cfg(test)]
mod test_util;
mod ui;

use std::io;

use chrono::Local;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::app::App;
use crate::store::Store;

fn main() -> io::Result<()> {
    let today = Local::now().date_naive();
    let mut store = Store::load()?;
    store.roll_over(today)?;
    let mut app = App::new(store, today);
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key)?;
        }
    }
    Ok(())
}
