use std::{
    fmt,
    fs::File,
    io::{self, Read, Seek},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use crossbeam::{
    channel::{unbounded, Receiver, RecvError, SendError, Sender},
    select,
};
use notify::{event::ModifyKind, RecursiveMode, Watcher};

use crate::app::AppMessage;

struct RemoteFileReader {
    content_sender: Sender<io::Result<String>>,
    receiver: Receiver<()>,
    file_path: PathBuf,
    interval: Duration,
    remote: String,
    ssh_options: Option<String>,
}

impl RemoteFileReader {
    fn new(
        content_sender: Sender<io::Result<String>>,
        receiver: Receiver<()>,
        file_path: PathBuf,
        interval: Duration,
        remote: String,
        ssh_options: Option<String>,
    ) -> Self {
        RemoteFileReader {
            content_sender,
            receiver,
            file_path,
            interval,
            remote,
            ssh_options,
        }
    }

    fn run(&mut self) -> Result<(), ()> {
        loop {
            self.update().map_err(|_| ())?;
            select! {
                recv(self.receiver) -> msg => {
                    msg.map_err(|_| ())?;
                }
                default(self.interval) => {}
            }
        }
    }

    fn update(&mut self) -> Result<(), SendError<io::Result<String>>> {
        let mut ssh_args = Vec::new();
        if let Some(ssh_options) = &self.ssh_options {
            ssh_args.extend(ssh_options.split_whitespace());
        }
        ssh_args.push(&self.remote);
        ssh_args.push("cat");
        ssh_args.push(self.file_path.to_str().unwrap());

        let mut cmd = Command::new("ssh");
        cmd.args(ssh_args);

        let output = cmd.output();

        let s = output.and_then(|o| {
            if o.status.success() {
                Ok(String::from_utf8_lossy(&o.stdout).to_string())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    String::from_utf8_lossy(&o.stderr).to_string(),
                ))
            }
        });

        self.content_sender.send(s)
    }
}

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
    remote: Option<String>,
    ssh_options: Option<String>,
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
        remote: Option<String>,
        ssh_options: Option<String>,
    ) -> Self {
        FileWatcher {
            app: app,
            receiver: receiver,
            file_path: None,
            interval: interval,
            remote,
            ssh_options,
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
                                if self.remote.is_none() {
                                    watcher.unwatch(p).expect(format!("Failed to unwatch {:?}", p).as_str());
                                }
                                self.file_path = None;
                            }

                            if let Some(p) = file_path {
                                if let Some(remote) = self.remote.clone() {
                                    let i = self.interval.clone();
                                    let ssh_options = self.ssh_options.clone();
                                    thread::spawn(move || RemoteFileReader::new(_content_sender, _watch_receiver, p, i, remote, ssh_options).run());
                                } else {
                                    let res = watcher.watch(Path::new(&p), RecursiveMode::NonRecursive);
                                    match res {
                                        Ok(_) => {
                                            self.file_path = Some(p.clone());
                                            let i = self.interval.clone();
                                            thread::spawn(move || FileReader::new(_content_sender, _watch_receiver, p, i).run());
                                        },
                                        Err(e) => self.app.send(AppMessage::JobOutput(Err(FileWatcherError::Watcher(e)))).unwrap()
                                    };
                                }
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
    pub fn new(
        app: Sender<AppMessage>,
        interval: Duration,
        remote: Option<String>,
        ssh_options: Option<String>,
    ) -> Self {
        let (sender, receiver) = unbounded();
        let mut actor = FileWatcher::new(app, receiver, interval, remote, ssh_options);
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