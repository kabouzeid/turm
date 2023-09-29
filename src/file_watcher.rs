use std::{
    fmt,
    fs::File,
    io::{self, Read, Seek},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crossbeam::{
    channel::{unbounded, Receiver, RecvError, SendError, Sender},
    select,
};
use notify::{event::ModifyKind, RecursiveMode, Watcher};
use tempfile::NamedTempFile;

use crate::app::AppMessage;

struct FileReader {
    content_sender: Sender<io::Result<String>>,
    receiver: Receiver<()>,
    file_path: PathBuf,
    interval: Duration,
    content: String,
    pos: u64,
}

struct FileWatcher {
    app: Sender<AppMessage>,
    receiver: Receiver<FileWatcherMessage>,
    file_path: Option<PathBuf>,
    interval: Duration,
}
pub enum FileWatcherMessage {
    FilePath(Option<PathBuf>),
}

pub struct FileWatcherHandle {
    sender: Sender<FileWatcherMessage>,
    file_path: Option<PathBuf>,
}

pub enum FileWatcherError {
    Watcher(notify::Error),
    File(io::Error),
}

impl fmt::Display for FileWatcherError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FileWatcherError::Watcher(e) => write!(f, "Watcher error: {}", e),
            FileWatcherError::File(e) => write!(f, "Read error: {}", e),
        }
    }
}

impl FileWatcher {
    fn new(
        app: Sender<AppMessage>,
        receiver: Receiver<FileWatcherMessage>,
        interval: Duration,
    ) -> Self {
        FileWatcher {
            app: app,
            receiver: receiver,
            file_path: None,
            interval: interval,
        }
    }

    fn run(&mut self) -> Result<(), RecvError> {
        let (watch_sender, watch_receiver) = unbounded();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let event = match res {
                Ok(e) => e,
                Err(_) => return,
            };
            match event.kind {
                notify::EventKind::Modify(ModifyKind::Data(_)) => {
                    watch_sender.send(event.paths).unwrap();
                }
                _ => {}
            };
        })
        .unwrap();

        // Check if the file watcher limit is reached by creating a file, watching it and then deleting it
        let tmp_file = NamedTempFile::new().unwrap();
        let tmp_file_path = tmp_file.path();
        let watcher_result = watcher.watch(tmp_file_path, RecursiveMode::NonRecursive);
        let max_watches_reached = match watcher_result {
            Err(notify::Error {
                kind: notify::ErrorKind::MaxFilesWatch,
                ..
            }) => true,
            Ok(_) => {
                watcher.unwatch(tmp_file_path).unwrap();
                false
            }
            _ => false,
        };
        let _ = tmp_file.close();

        let config = notify::Config::default()
            .with_poll_interval(Duration::from_secs(2))
            .with_compare_contents(false);

        let mut watcher = if max_watches_reached {
            let (watch_sender, _) = unbounded();
            let watcher = notify::PollWatcher::new(
                move |res: notify::Result<notify::Event>| {
                    let event = match res {
                        Ok(e) => e,
                        Err(_) => return,
                    };
                    match event.kind {
                        notify::EventKind::Modify(ModifyKind::Data(_)) => {
                            let _ = watch_sender.send(event.paths);
                        }
                        _ => {}
                    };
                },
                config,
            );
            Box::new(watcher.unwrap()) as Box<dyn Watcher>
        } else {
            Box::new(watcher) as Box<dyn Watcher>
        };

        let (mut _content_sender, mut _content_receiver) = unbounded::<io::Result<String>>();
        let (mut _watch_sender, mut _watch_receiver) = unbounded::<()>();
        loop {
            select! {
                recv(self.receiver) -> msg => {
                    match msg? {
                        FileWatcherMessage::FilePath(file_path) => {
                            (_content_sender, _content_receiver) = unbounded();
                            (_watch_sender, _watch_receiver) = unbounded::<()>();

                            if let Some(p) = &self.file_path {
                                let _ = watcher.unwatch(p);
                                self.file_path = None;
                            }

                            if let Some(p) = file_path {
                                let res = watcher.watch(Path::new(&p), RecursiveMode::NonRecursive);
                                match res {
                                    Ok(_) => {
                                        self.file_path = Some(p.clone());
                                        let i = self.interval.clone();
                                        thread::spawn(move || FileReader::new(_content_sender, _watch_receiver, p, i).run());
                                    },
                                    Err(e) => self.app.send(AppMessage::JobOutput(Err(FileWatcherError::Watcher(e)))).unwrap()
                                };
                            } else {
                                _content_sender.send(Ok("".to_string())).unwrap();
                            }
                        }
                    }
                }
                recv(watch_receiver) -> _ => { _watch_sender.send(()).unwrap(); }
                recv(_content_receiver) -> msg => {
                    self.app.send(AppMessage::JobOutput(msg.unwrap().map_err(|e| FileWatcherError::File(e)))).unwrap();
                }
            }
        }
    }
}

impl FileReader {
    fn new(
        content_sender: Sender<io::Result<String>>,
        receiver: Receiver<()>,
        file_path: PathBuf,
        interval: Duration,
    ) -> Self {
        FileReader {
            content_sender: content_sender,
            receiver: receiver,
            file_path: file_path,
            interval: interval,
            content: "".to_string(),
            pos: 0,
        }
    }

    fn run(&mut self) -> Result<(), ()> {
        loop {
            self.update().map_err(|_| ())?;
            select! {
                recv(self.receiver) -> msg => {
                    msg.map_err(|_| ())?;
                }
                // in case the file watcher doesn't work (e.g. network mounted fs)
                default(self.interval) => {}
            }
        }
    }

    fn update(&mut self) -> Result<(), SendError<io::Result<String>>> {
        let s = File::open(&self.file_path).and_then(|mut f| {
            // avoid reading the whole file every time
            self.pos = f.seek(io::SeekFrom::Start(self.pos))?;
            self.pos += f.read_to_string(&mut self.content)? as u64;
            Ok(self.content.clone())
        });
        // let s = fs::read_to_string(&self.file_path); // alternative: always read the whole file
        self.content_sender.send(s)
    }
}

impl FileWatcherHandle {
    pub fn new(app: Sender<AppMessage>, interval: Duration) -> Self {
        let (sender, receiver) = unbounded();
        let mut actor = FileWatcher::new(app, receiver, interval);
        thread::spawn(move || actor.run());

        Self {
            sender,
            file_path: None,
        }
    }

    pub fn set_file_path(&mut self, file_path: Option<PathBuf>) {
        if self.file_path != file_path {
            self.file_path = file_path.clone();
            let _ = self.sender.send(FileWatcherMessage::FilePath(file_path));
        }
    }
}
