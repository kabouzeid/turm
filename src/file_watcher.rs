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

struct FileWatcher {
    app: Sender<AppMessage>,
    receiver: Receiver<FileWatcherMessage>,
    file_path: Option<notify::Result<PathBuf>>,
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

        loop {
            select! {
                recv(self.receiver) -> msg => {
                    match msg.unwrap() {
                        FileWatcherMessage::FilePath(file_path) => {
                            if let Some(Ok(p)) = &self.file_path {
                                watcher.unwatch(p).expect(format!("Failed to unwatch {:?}", p).as_str());
                                self.file_path = None;
                            }
                            self.file_path = file_path.map(|p| watcher.watch(Path::new(&p), RecursiveMode::NonRecursive).map(|_| p));
                        }
                    }
                }
                recv(watch_receiver) -> _ => {}
                // in case the file watcher doesn't work (e.g. network mounted fs)
                default(Duration::from_secs(5)) => {}
            }
            self.update();
        }
    }

    fn update(&self) {
        let s = self.file_path.as_ref().and_then(|file_path| {
            match file_path {
                Ok(p) => {
                    // TODO: partial read only
                    fs::read_to_string(p).ok().map(|s| s.replace("\r", "\n")) // .replace doesn't really belong here, but it can be slow because log files can be huge => so better to do it here (off the UI thread) for now
                }
                Err(e) => Some(format!("Error: {}", e)),
            }
        });
        self.app.send(AppMessage::JobStdout(s)).unwrap();
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
