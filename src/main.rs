mod app;
mod file_watcher;
mod job_watcher;

use app::{App};
use crossbeam::{
    channel::{unbounded, Sender},
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{io, thread};
use tui::{
    backend::{Backend, CrosstermBackend}, Terminal,
};

fn main() -> Result<(), io::Error> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    run_app(&mut terminal)?;

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn input_loop(tx: Sender<crossterm::Result<Event>>) {
    loop {
        tx.send(event::read()).unwrap();
    }
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>) -> io::Result<()> {
    let (input_tx, input_rx) = unbounded();
    let mut app = App::new(input_rx);
    thread::spawn(move || input_loop(input_tx));
    app.run(terminal)
}
