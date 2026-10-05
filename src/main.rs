mod app;
mod store;
mod ui;

use std::io;

use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::app::App;
use crate::store::Store;

fn main() -> io::Result<()> {
    let mut app = App::new(Store::load()?);
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
