use crossbeam::{
    channel::{unbounded, Receiver},
    select,
};
use std::path::PathBuf;
use std::time::Duration;

use crate::file_watcher::{FileWatcherError, FileWatcherHandle};
use crate::job_watcher::JobWatcherHandle;

use crossterm::event::{Event, KeyCode, KeyEvent};
use std::io;
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

pub enum Focus {
    Jobs,
}

pub struct App {
    focus: Focus,
    jobs: Vec<Job>,
    job_list_state: ListState,
    job_stdout: Result<String, FileWatcherError>,
    job_stdout_offset: u16,
    _job_watcher: JobWatcherHandle,
    job_stdout_watcher: FileWatcherHandle,
    // sender: Sender<AppMessage>,
    receiver: Receiver<AppMessage>,
    input_receiver: Receiver<crossterm::Result<Event>>,
}

pub struct Job {
    pub id: String,
    pub name: String,
    pub state: String,
    pub state_compact: String,
    pub reason: Option<String>,
    pub user: String,
    pub time: String,
    pub tres: String,
    pub partition: String,
    pub nodelist: String,
    pub stdout: Option<PathBuf>,
    pub command: String,
    // pub stderr: Option<PathBuf>,
}

pub enum AppMessage {
    Jobs(Vec<Job>),
    JobStdout(Result<String, FileWatcherError>),
    Key(KeyEvent),
}

impl App {
    pub fn new(input_receiver: Receiver<crossterm::Result<Event>>) -> App {
        let (sender, receiver) = unbounded();
        Self {
            focus: Focus::Jobs,
            jobs: Vec::new(),
            _job_watcher: JobWatcherHandle::new(sender.clone(), Duration::from_secs(2)),
            job_list_state: ListState::default(),
            job_stdout: Ok("".to_string()),
            job_stdout_offset: 0,
            job_stdout_watcher: FileWatcherHandle::new(sender.clone(), Duration::from_secs(2)),
            // sender,
            receiver: receiver,
            input_receiver: input_receiver,
        }
    }
}

impl App {
    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> io::Result<()> {
        terminal.draw(|f| self.ui(f))?;

        loop {
            select! {
                recv(self.receiver) -> event => {
                    self.handle(event.unwrap());
                }
                recv(self.input_receiver) -> input_res => {
                    match input_res.unwrap().unwrap() {
                        Event::Key(key) => {
                            if key.code == KeyCode::Char('q') {
                                return Ok(());
                            }
                            self.handle(AppMessage::Key(key));
                        },
                        Event::Resize(_, _) => {},
                        _ => continue, // ignore and do not redraw
                    }
                }
            };

            terminal.draw(|f| self.ui(f))?;
        }
    }

    fn handle(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::Jobs(jobs) => self.jobs = jobs,
            AppMessage::JobStdout(content) => self.job_stdout = content,
            AppMessage::Key(key) => {
                match key.code {
                    KeyCode::Char('h') | KeyCode::Left => self.focus_previous_panel(),
                    KeyCode::Char('l') | KeyCode::Right => self.focus_next_panel(),
                    KeyCode::Char('k') | KeyCode::Up => match self.focus {
                        Focus::Jobs => self.select_previous_job(),
                    },
                    KeyCode::Char('j') | KeyCode::Down => match self.focus {
                        Focus::Jobs => self.select_next_job(),
                    },
                    KeyCode::PageDown => {
                        self.job_stdout_offset = self.job_stdout_offset.saturating_sub(
                            if key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                            {
                                10
                            } else {
                                1
                            },
                        )
                    }
                    KeyCode::PageUp => {
                        self.job_stdout_offset = self.job_stdout_offset.saturating_add(
                            if key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                            {
                                10
                            } else {
                                1
                            },
                        )
                    }
                    KeyCode::End => self.job_stdout_offset = 0,
                    // KeyCode::Home => {
                    //     // somehow scroll to top?
                    // }
                    _ => {}
                };
            }
        }

        // update
        self.job_stdout_watcher.set_file_path(
            self.job_list_state
                .selected()
                .and_then(|i| self.jobs.get(i).and_then(|j| j.stdout.clone())),
        );
    }

    fn ui<B: Backend>(&mut self, f: &mut Frame<B>) {
        // Layout

        let content_help = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)].as_ref())
            .split(f.size());

        let master_detail = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(50), Constraint::Percentage(70)].as_ref())
            .split(content_help[0]);

        let job_detail_log = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Min(3)].as_ref())
            .split(master_detail[1]);

        // Help

        let help = Spans::from(vec![
            // ⏴⏵⏶⏷
            Span::styled("⏶/⏷", Style::default().fg(Color::Blue)),
            Span::styled(": navigate", Style::default().fg(Color::LightBlue)),
            Span::raw(" | "),
            Span::styled("pgup/pgdown/end", Style::default().fg(Color::Blue)),
            Span::styled(": scroll", Style::default().fg(Color::LightBlue)),
            Span::raw(" | "),
            Span::styled("ctrl", Style::default().fg(Color::Blue)),
            Span::styled(": fast scroll", Style::default().fg(Color::LightBlue)),
            Span::raw(" | "),
            Span::styled("q", Style::default().fg(Color::Blue)),
            Span::styled(": quit", Style::default().fg(Color::LightBlue)),
        ]);
        let help = Paragraph::new(help);
        f.render_widget(help, content_help[1]);

        // Jobs
        let max_user_len = self.jobs.iter().map(|j| j.user.len()).max().unwrap_or(0);
        let max_partition_len = self
            .jobs
            .iter()
            .map(|j| j.partition.len())
            .max()
            .unwrap_or(0);
        let max_time_len = self.jobs.iter().map(|j| j.time.len()).max().unwrap_or(0);
        let max_state_compact_len = self
            .jobs
            .iter()
            .map(|j| j.state_compact.len())
            .max()
            .unwrap_or(0);
        let jobs: Vec<ListItem> = self
            .jobs
            .iter()
            .map(|j| {
                ListItem::new(Spans::from(vec![
                    Span::styled(
                        format!(
                            "{:<max$.max$}",
                            j.state_compact,
                            max = max_state_compact_len
                        ),
                        Style::default(),
                    ),
                    Span::raw(" "),
                    Span::styled(&j.id, Style::default().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:<max$.max$}", j.partition, max = max_partition_len),
                        Style::default().fg(Color::Blue),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:<max$.max$}", j.user, max = max_user_len),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{:>max$.max$}", j.time, max = max_time_len),
                        Style::default().fg(Color::Red),
                    ),
                    Span::raw(" "),
                    Span::raw(&j.name),
                ]))
            })
            .collect();
        let job_list = List::new(jobs)
            .block(
                Block::default()
                    .title("Job Queue")
                    .borders(Borders::ALL)
                    .border_style(match self.focus {
                        Focus::Jobs => Style::default().fg(Color::Green),
                    }),
            )
            .highlight_style(Style::default().bg(Color::Green).fg(Color::Black));
        f.render_stateful_widget(job_list, master_detail[0], &mut self.job_list_state);

        // Job details

        let job_detail = self
            .job_list_state
            .selected()
            .and_then(|i| self.jobs.get(i));

        let job_detail = job_detail.map(|j| {
            let state = Spans::from(vec![
                Span::styled("State  ", Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::raw(&j.state),
                if let Some(s) = j.reason.as_deref() {
                    Span::styled(
                        format!(" ({s})"),
                        Style::default().add_modifier(Modifier::DIM),
                    )
                } else {
                    Span::raw("")
                },
            ]);

            let command = Spans::from(vec![
                Span::styled("Command", Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::raw(&j.command),
            ]);
            let nodes = Spans::from(vec![
                Span::styled("Nodes  ", Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::raw(&j.nodelist),
            ]);
            let tres = Spans::from(vec![
                Span::styled("TRES   ", Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::raw(&j.tres),
            ]);
            let stdout = Spans::from(vec![
                Span::styled("stdout ", Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::raw(
                    j.stdout
                        .as_ref()
                        .map(|p| p.to_str().unwrap_or_default())
                        .unwrap_or_default(),
                ),
            ]);

            Text::from(vec![state, command, nodes, tres, stdout])
        });
        let job_detail = Paragraph::new(job_detail.unwrap_or_default())
            .block(Block::default().title("Details").borders(Borders::ALL));
        f.render_widget(job_detail, job_detail_log[0]);

        // Log
        let log_area = job_detail_log[1];
        let log_title = Spans::from(vec![
            Span::raw("stdout"),
            Span::raw(if self.job_stdout_offset > 0 {
                format!("[{}]", self.job_stdout_offset)
            } else {
                "".to_string()
            }),
        ]);
        let log_block = Block::default().title(log_title).borders(Borders::ALL);

        // let job_log = self.job_stdout.as_deref().map(|s| {
        //     string_for_paragraph(
        //         s,
        //         log_block.inner(log_area).height as usize,
        //         log_block.inner(log_area).width as usize,
        //         self.job_stdout_offset as usize,
        //     )
        // }).unwrap_or_else(|e| {
        //     self.job_stdout_offset = 0;
        //     "".to_string()
        // });

        let log = match self.job_stdout.as_deref() {
            Ok(s) => Paragraph::new(string_for_paragraph(
                s,
                log_block.inner(log_area).height as usize,
                log_block.inner(log_area).width as usize,
                self.job_stdout_offset as usize,
            )),
            Err(e) => Paragraph::new(e.to_string())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true }),
        }
        .block(log_block);

        f.render_widget(log, log_area);
    }
}

fn string_for_paragraph(s: &str, lines: usize, cols: usize, offset: usize) -> String {
    s.lines()
        .flat_map(|l| l.split('\r')) // bandaid for term escape codes
        .rev()
        .skip(offset)
        .take(lines)
        .map(|l| l.chars().take(cols).collect::<String>())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

impl App {
    fn focus_next_panel(&mut self) {
        match self.focus {
            Focus::Jobs => self.focus = Focus::Jobs,
        }
    }

    fn focus_previous_panel(&mut self) {
        match self.focus {
            Focus::Jobs => self.focus = Focus::Jobs,
        }
    }

    fn select_next_job(&mut self) {
        let i = match self.job_list_state.selected() {
            Some(i) => {
                if i >= self.jobs.len() - 1 {
                    self.jobs.len() - 1
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.job_list_state.select(Some(i));
    }

    fn select_previous_job(&mut self) {
        let i = match self.job_list_state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.job_list_state.select(Some(i));
    }
}
