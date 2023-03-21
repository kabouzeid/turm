use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crossbeam::{
    channel::{unbounded, Receiver, Sender},
    select,
};
use notify::{event::ModifyKind, RecursiveMode, Watcher};

use crate::app::AppMessage;

struct FileReader {
    content_sender: Sender<Option<String>>,
    receiver: Receiver<()>,
    file_path: Option<PathBuf>,
}

struct FileWatcher {
    app: Sender<AppMessage>,
    receiver: Receiver<FileWatcherMessage>,
    file_path: Option<PathBuf>,
}
pub enum FileWatcherMessage {
    FilePath(Option<PathBuf>),
}

pub struct FileWatcherHandle {
    sender: Sender<FileWatcherMessage>,
    file_path: Option<PathBuf>,
}

impl FileWatcher {
    fn new(app: Sender<AppMessage>, receiver: Receiver<FileWatcherMessage>) -> Self {
        FileWatcher {
            app: app,
            receiver: receiver,
            file_path: None,
        }
    }

    fn run(&mut self) {
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

        let (mut content_sender, mut content_receiver) = unbounded::<Option<String>>();
        let (mut _watch_sender, mut _watch_receiver) = unbounded::<()>();
        loop {
            select! {
                recv(self.receiver) -> msg => {
                    match msg.unwrap() {
                        FileWatcherMessage::FilePath(file_path) => {
                            (content_sender, content_receiver) = unbounded();
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
                                        thread::spawn(move || FileReader::new(content_sender, _watch_receiver, p).run());
                                    },
                                    Err(e) => self.app.send(AppMessage::JobStdout(Some(format!("Failed to watch {:?}: {}", p, e)))).unwrap()
                                };
                            }
                        }
                    }
                }
                recv(watch_receiver) -> _ => { _watch_sender.send(()).unwrap(); }
                recv(content_receiver) -> msg => {
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
    ) -> Self {
        FileReader {
            content_sender: content_sender,
            receiver: receiver,
            file_path: file_path,
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
                default(Duration::from_secs(10)) => {}
            }
        }
    }

    fn update(&self) -> Result<(), crossbeam::channel::SendError<Option<String>>> {
        let s = self.file_path.as_ref().and_then(|file_path| {
            // TODO: partial read only
            fs::read_to_string(file_path)
                .ok()
                .map(|s| s.replace("\r", "\n")) // .replace doesn't really belong here, but it can be slow because log files can be huge => so better to do it here (off the UI thread) for now
        });
        self.content_sender.send(s)
    }
}

impl FileWatcherHandle {
    pub fn new(app: Sender<AppMessage>) -> Self {
        let (sender, receiver) = unbounded();
        let mut actor = FileWatcher::new(app, receiver);
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
