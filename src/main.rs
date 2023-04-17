mod app;
mod file_watcher;
mod job_watcher;
mod squeue_args;

use app::App;
use clap::Parser;
use crossbeam::channel::{unbounded, Sender};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use squeue_args::SqueueArgs;
use std::{io, thread};
use tui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Refresh rate for the job watcher.
    #[arg(long, value_name = "SECONDS", default_value_t = 2)]
    slurm_refresh: u64,

    /// Refresh rate for the file watcher.
    #[arg(long, value_name = "SECONDS", default_value_t = 2)]
    file_refresh: u64,

    /// squeue arguments
    #[command(flatten)]
    squeue_args: SqueueArgs,
}

fn main() -> Result<(), io::Error> {
    let args = Cli::parse();

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    run_app(&mut terminal, args)?;

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

fn run_app<B: Backend>(terminal: &mut Terminal<B>, args: Cli) -> io::Result<()> {
    let (input_tx, input_rx) = unbounded();
    let mut app = App::new(
        input_rx,
        args.slurm_refresh,
        args.file_refresh,
        args.squeue_args.to_vec(),
    );
    thread::spawn(move || input_loop(input_tx));
    app.run(terminal)
}
