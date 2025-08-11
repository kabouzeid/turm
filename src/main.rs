mod app;
mod file_watcher;
mod job_watcher;
mod squeue_args;

use app::App;
use clap::CommandFactory;
use clap::Parser;
use clap::Subcommand;
use clap_complete::{generate, Shell};
use crossbeam::channel::{unbounded, Sender};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use squeue_args::SqueueArgs;
use std::{io, thread};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Refresh rate for the job watcher.
    #[arg(long, value_name = "SECONDS", default_value_t = 2)]
    slurm_refresh: u64,

    /// Refresh rate for the file watcher.
    #[arg(long, value_name = "SECONDS", default_value_t = 2)]
    file_refresh: u64,

    /// Run slurm commands on a remote host via ssh.
    #[arg(long)]
    remote: Option<String>,

    /// Extra options to pass to the ssh command.
    #[arg(long, value_name = "OPTIONS")]
    ssh_options: Option<String>,

    /// squeue arguments
    #[command(flatten)]
    squeue_args: SqueueArgs,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Print shell completion script to stdout.
    Completion {
        /// The shell to generate completion for.
        shell: Shell,
    },
}

fn main() -> Result<(), io::Error> {
    let args = Cli::parse();
    match args.command {
        Some(CliCommand::Completion { shell }) => {
            let cmd = &mut Cli::command();
            generate(shell, cmd, cmd.get_name().to_string(), &mut io::stdout());
            return Ok(());
        }
        None => {}
    }

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

fn input_loop(tx: Sender<std::io::Result<Event>>) {
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
        args.remote,
        args.ssh_options,
    );
    thread::spawn(move || input_loop(input_tx));
    app.run(terminal)
}
