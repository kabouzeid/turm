use std::io::Lines;
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
}

pub struct JobWatcherHandle {}

fn addr_of(s: &str) -> usize {
    s.as_ptr() as usize
}

fn split_whitespace_indices(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.split_whitespace()
        .map(move |sub| (addr_of(sub) - addr_of(s), sub))
}

fn offsets_of(s: String) -> Vec<usize> {
    let iter = split_whitespace_indices(&s);
    let mut indices: Vec<usize> = iter.map(|split| split.0).collect();
    indices.push(s.len());
    return indices;
}

impl JobWatcher {
    fn new(app: Sender<AppMessage>, interval: Duration, squeue_args: Vec<String>) -> Self {
        Self {
            app,
            interval,
            squeue_args,
        }
    }

    /// Run Slurm squeue compatible with legacy Slurm versions
    ///
    /// API compatible with `run`. Uses column header index offsets instead of a magic
    /// string splitter.
    fn run(&mut self) -> Self {
        let fields = [
            "jobid",
            "name",
            "state",
            "username",
            "timeused",
            "tres-alloc:80",
            "partition",
            "nodelist",
            "stdout:80",
            "stderr:80",
            "command:80",
            "statecompact",
            "reason",
            "ArrayJobID",  // %A
            "ArrayTaskID", // %a
            "NodeList",    // %N
            "WorkDir",     // for fallback
        ];
        let output_format = fields.map(|s| s.to_owned()).join(",");

        loop {
            let results = Command::new("squeue")
                .args(&self.squeue_args)
                .arg("--array")
                .arg("--Format")
                .arg(&output_format)
                .output()
                .expect("failed to execute process");
            let mut lines: Lines<&[u8]> = results.stdout.lines();
            let first_line = lines.next().unwrap().unwrap();
            let offsets = offsets_of(first_line);
            let jobs: Vec<Job> = lines
                .map(|l| l.unwrap().trim().to_string())
                .filter_map(|l| {
                    let mut parts: Vec<_> = (0..offsets.len() - 1)
                        .map(|i| l[offsets[i]..offsets[i + 1]].trim())
                        .collect();
                    let last_offset: usize = offsets[offsets.len() - 1];
                    parts.push(&l[last_offset..l.len() - 1]);

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
    pub fn new(app: Sender<AppMessage>, interval: Duration, squeue_args: Vec<String>) -> Self {
        let mut actor = JobWatcher::new(app, interval, squeue_args);
        thread::spawn(move || actor.run());

        Self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offsets_of() {
        let s = b"JOBID               NAME                STATE               USER                TIME                TRES_ALLOC          PARTITION           NODELIST            STDOUT                                                                                              STDERR              COMMAND             ST                  REASON              ARRAY_JOB_ID        ARRAY_TASK_ID       NODELIST            WORK_DIR";
        let mut lines = s.lines();
        let first_line = lines.next().unwrap().unwrap();
        let results = offsets_of(first_line);
        assert_eq!(
            results,
            [0, 20, 40, 60, 80, 100, 120, 140, 160, 260, 280, 300, 320, 340, 360, 380, 400, 408]
        );
    }
}
