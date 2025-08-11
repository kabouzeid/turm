use std::path::PathBuf;
use std::{io::BufRead, process::Command, thread, time::Duration};

use crossbeam::channel::Sender;
use regex::Regex;

use crate::app::AppMessage;
use crate::app::Job;

struct JobWatcher {
    app: Sender<AppMessage>,
    interval: Duration,
    squeue_args: Vec<String>,
    remote: Option<String>,
    ssh_options: Option<String>,
}

pub struct JobWatcherHandle {}

impl JobWatcher {
    fn new(
        app: Sender<AppMessage>,
        interval: Duration,
        squeue_args: Vec<String>,
        remote: Option<String>,
        ssh_options: Option<String>,
    ) -> Self {
        Self {
            app,
            interval,
            squeue_args,
            remote,
            ssh_options,
        }
    }

    fn run(&mut self) -> Self {
        let output_separator = "###turm###";
        let fields = [
            "jobid",
            "name",
            "state",
            "username",
            "timeused",
            "tres-alloc",
            "partition",
            "nodelist",
            "stdout",
            "stderr",
            "command",
            "statecompact",
            "reason",
            "ArrayJobID",  // %A
            "ArrayTaskID", // %a
            "NodeList",    // %N
            "WorkDir",     // for fallback
        ];
        let output_format = fields
            .map(|s| s.to_owned() + ":" + output_separator)
            .join(",");

        loop {
            let mut command = if let Some(remote) = &self.remote {
                let mut ssh_args = Vec::new();
                if let Some(ssh_options) = &self.ssh_options {
                    ssh_args.extend(ssh_options.split_whitespace().map(|s| s.to_string()));
                }
                ssh_args.push(remote.to_string());
                ssh_args.push("squeue".to_string());
                ssh_args.extend_from_slice(&self.squeue_args);
                ssh_args.push("--array".to_string());
                ssh_args.push("--noheader".to_string());
                ssh_args.push("--Format".to_string());
                ssh_args.push(output_format.clone());
                let mut cmd = Command::new("ssh");
                cmd.args(ssh_args);
                cmd
            } else {
                let mut cmd = Command::new("squeue");
                cmd.args(&self.squeue_args)
                    .arg("--array")
                    .arg("--noheader")
                    .arg("--Format")
                    .arg(&output_format);
                cmd
            };

            let output = command.output();

            match output {
                Ok(output) => {
                    if output.status.success() {
                        let jobs: Vec<Job> = output
                            .stdout
                            .lines()
                            .map(|l| l.unwrap().trim().to_string())
                            .filter_map(|l| {
                                let parts: Vec<_> = l.split(output_separator).collect();

                                if parts.len() != fields.len() + 1 {
                                    return None;
                                }

                                let id = parts[0];
                                let name = parts[1];
                                let state = parts[2];
                                let user = parts[3];
                                let time = parts[4];
                                let tres = parts[5];
                                let partition = parts[6];
                                let nodelist = parts[7];
                                let stdout = parts[8];
                                let stderr = parts[9];
                                let command = parts[10];
                                let state_compact = parts[11];
                                let reason = parts[12];

                                let array_job_id = parts[13];
                                let array_task_id = parts[14];
                                let node_list = parts[15];
                                let working_dir = parts[16];

                                Some(Job {
                                    job_id: id.to_owned(),
                                    array_id: array_job_id.to_owned(),
                                    array_step: match array_task_id {
                                        "N/A" => None,
                                        _ => Some(array_task_id.to_owned()),
                                    },
                                    name: name.to_owned(),
                                    state: state.to_owned(),
                                    state_compact: state_compact.to_owned(),
                                    reason: if reason == "None" {
                                        None
                                    } else {
                                        Some(reason.to_owned())
                                    },
                                    user: user.to_owned(),
                                    time: time.to_owned(),
                                    tres: tres.to_owned(),
                                    partition: partition.to_owned(),
                                    nodelist: nodelist.to_owned(),
                                    command: command.to_owned(),
                                    stdout: Self::resolve_path(
                                        stdout,
                                        array_job_id,
                                        array_task_id,
                                        id,
                                        node_list,
                                        user,
                                        name,
                                        working_dir,
                                    ),
                                    stderr: Self::resolve_path(
                                        stderr,
                                        array_job_id,
                                        array_task_id,
                                        id,
                                        node_list,
                                        user,
                                        name,
                                        working_dir,
                                    ), // TODO fill all fields
                                })
                            })
                            .collect();
                        self.app.send(AppMessage::Jobs(jobs)).unwrap();
                    } else {
                        let err = String::from_utf8_lossy(&output.stderr).to_string();
                        self.app.send(AppMessage::Error(err)).unwrap();
                    }
                }
                Err(err) => {
                    self.app
                        .send(AppMessage::Error(err.to_string()))
                        .unwrap();
                }
            }
            thread::sleep(self.interval);
        }
    }

    fn resolve_path(
        path: &str,
        array_master: &str,
        array_id: &str,
        id: &str,
        host: &str,
        user: &str,
        name: &str,
        working_dir: &str,
    ) -> Option<PathBuf> {
        // see https://slurm.schedmd.com/sbatch.html#SECTION_%3CB%3Efilename-pattern%3C/B%3E
        lazy_static::lazy_static! {
            static ref RE: Regex = Regex::new(r"%(%|A|a|J|j|N|n|s|t|u|x)").unwrap();
        }

        let mut path = path.to_owned();
        let slurm_no_val = "4294967294";
        let array_id = if array_id == "N/A" {
            slurm_no_val
        } else {
            array_id
        };

        if path.is_empty() {
            // never happens right now, because `squeue -O stdout` seems to always return something
            path = if array_id == slurm_no_val {
                PathBuf::from(working_dir).join("slurm-%J.out")
            } else {
                PathBuf::from(working_dir).join("slurm-%A_%a.out")
            }
            .to_str()
            .unwrap()
            .to_owned();
        };

        for cap in RE
            .captures_iter(&path.clone())
            .collect::<Vec<_>>() // TODO: this is stupid, there has to be a better way to reverse the captures...
            .iter()
            .rev()
        {
            let m = cap.get(0).unwrap();
            let replacement = match m.as_str() {
                "%%" => "%",
                "%A" => array_master,
                "%a" => array_id,
                "%J" => id,
                "%j" => id,
                "%N" => host.split(',').next().unwrap_or(host),
                "%n" => "0",
                "%s" => "batch",
                "%t" => "0",
                "%u" => user,
                "%x" => name,
                _ => unreachable!(),
            };

            path.replace_range(m.range(), replacement);
        }

        Some(PathBuf::from(working_dir).join(path)) // works even if `path` is absolute
    }
}

impl JobWatcherHandle {
    pub fn new(
        app: Sender<AppMessage>,
        interval: Duration,
        squeue_args: Vec<String>,
        remote: Option<String>,
        ssh_options: Option<String>,
    ) -> Self {
        let mut actor = JobWatcher::new(app, interval, squeue_args, remote, ssh_options);
        thread::spawn(move || actor.run());

        Self {}
    }
}
