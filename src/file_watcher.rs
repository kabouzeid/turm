use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crossbeam::{
    channel::{unbounded, Receiver, RecvError, SendError, Sender},
    select,
};
use notify::{event::ModifyKind, RecursiveMode, Watcher};

use crate::app::AppMessage;

struct FileReader {
    content_sender: Sender<Option<String>>,
    receiver: Receiver<()>,
    file_path: Option<PathBuf>,
    interval: Duration,
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
            let event = res.unwrap();
            match event.kind {
                notify::EventKind::Modify(ModifyKind::Data(_)) => {
                    watch_sender.send(event.paths).unwrap();
                }
                _ => {}
            };
        })
        .unwrap();

        let (mut _content_sender, mut _content_receiver) = unbounded::<Option<String>>();
        let (mut _watch_sender, mut _watch_receiver) = unbounded::<()>();
        loop {
            select! {
                recv(self.receiver) -> msg => {
                    match msg? {
                        FileWatcherMessage::FilePath(file_path) => {
                            (_content_sender, _content_receiver) = unbounded();
                            (_watch_sender, _watch_receiver) = unbounded::<()>();

                            if let Some(p) = &self.file_path {
                                watcher.unwatch(p).expect(format!("Failed to unwatch {:?}", p).as_str());
                                self.file_path = None;
                            }

                            if let Some(p) = file_path {
                                let res = watcher.watch(Path::new(&p), RecursiveMode::NonRecursive);
                                match res {
                                    Ok(_) => {
                                        self.file_path = Some(p);
                                        let p = self.file_path.clone();
                                        let i = self.interval.clone();
                                        thread::spawn(move || FileReader::new(_content_sender, _watch_receiver, p, i).run());
                                    },
                                    Err(e) => self.app.send(AppMessage::JobStdout(Some(format!("Failed to watch {:?}: {}", p, e)))).unwrap()
                                };
                            }
                        }
                    }
                }
                recv(watch_receiver) -> _ => { _watch_sender.send(()).unwrap(); }
                recv(_content_receiver) -> msg => {
                    self.app.send(AppMessage::JobStdout(msg.unwrap())).unwrap();
                }
            }
        }
    }
}

impl FileReader {
    fn new(
        content_sender: Sender<Option<String>>,
        receiver: Receiver<()>,
        file_path: Option<PathBuf>,
        interval: Duration,
    ) -> Self {
        FileReader {
            content_sender: content_sender,
            receiver: receiver,
            file_path: file_path,
            interval: interval,
        }
    }

    fn run(&self) -> Result<(), ()> {
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

    fn update(&self) -> Result<(), SendError<Option<String>>> {
        let s = self.file_path.as_ref().and_then(|file_path| {
            // TODO: partial read only
            fs::read_to_string(file_path).ok()
        });
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
            self.sender
                .send(FileWatcherMessage::FilePath(file_path))
                .unwrap();
        }
    }
}
