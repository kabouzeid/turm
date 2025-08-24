use std::error::Error;
use std::path::PathBuf;
use std::{io::BufRead, process::Command, thread, time::Duration};

use crossbeam::channel::Sender;
use regex::Regex;

use crate::app::AppMessage;
use crate::app::Job;

// Enum to track which squeue command version to use.
#[derive(Clone, Copy, Debug)]
enum SqueueMethod {
    /// The method has not yet been determined.
    Unknown,
    /// Use the modern `squeue --Format` command.
    Modern,
    /// Use the legacy `squeue -o` command.
    Legacy,
}

struct JobWatcher {
    app: Sender<AppMessage>,
    interval: Duration,
    squeue_args: Vec<String>,
    method: SqueueMethod,
}

pub struct JobWatcherHandle {}

impl JobWatcher {
    fn new(app: Sender<AppMessage>, interval: Duration, squeue_args: Vec<String>) -> Self {
        Self {
            app,
            interval,
            squeue_args,
            method: SqueueMethod::Unknown, // Start with unknown, we will detect it on the first run.
        }
    }

    /// The main loop that orchestrates job fetching.
    /// It uses a state machine to determine which squeue method to use.
    fn run(&mut self) -> Self {
        loop {
            let jobs_result = match self.method {
                SqueueMethod::Unknown => {
                    // First time running, try to detect the best method.
                    // Attempt modern method first.
                    match self.fetch_and_parse_modern() {
                        Ok(jobs) => {
                            // eprintln!("Successfully detected modern `squeue --Format` method.");
                            self.method = SqueueMethod::Modern; // It worked! Lock it in.
                            Ok(jobs)
                        }
                        Err(_e) => {
                            // Modern method failed, fall back to legacy.
                            // eprintln!("Modern method failed ({}). Falling back to legacy `squeue -o` method.", e);
                            match self.fetch_and_parse_legacy() {
                                Ok(jobs) => {
                                    // eprintln!("Successfully detected legacy `squeue -o` method.");
                                    self.method = SqueueMethod::Legacy; // It worked! Lock it in.
                                    Ok(jobs)
                                }
                                Err(_e) => Err(_e), // Both failed, something is wrong.
                            }
                        }
                    }
                }
                SqueueMethod::Modern => self.fetch_and_parse_modern(),
                SqueueMethod::Legacy => self.fetch_and_parse_legacy(),
            };

            match jobs_result {
                Ok(jobs) => self.app.send(AppMessage::Jobs(jobs)).unwrap(),
                Err(_e) => {
                    // eprintln!("Error fetching jobs with method {:?}: {}", self.method, e);
                }
            }

            thread::sleep(self.interval);
        }
    }

    /// Fetches jobs using the modern `squeue --Format` method.
    fn fetch_and_parse_modern(&self) -> Result<Vec<Job>, Box<dyn Error>> {
        let output_separator = "###turm###";
        let fields = [
            "jobid", "name", "state", "username", "timeused", "tres-alloc", "partition",
            "nodelist", "stdout", "stderr", "command", "statecompact", "reason",
            "ArrayJobID", "ArrayTaskID", "NodeList", "WorkDir",
        ];
        let output_format = fields
            .map(|s| s.to_owned() + ":" + output_separator)
            .join(",");

        let output = Command::new("squeue")
            .args(&self.squeue_args)
            .arg("--array")
            .arg("--noheader")
            .arg("--Format")
            .arg(&output_format)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("squeue command failed with non-zero status. Stderr: {}", stderr).into());
        }

        let jobs: Vec<Job> = output
            .stdout
            .lines()
            .filter_map(|l| {
                let line = l.ok()?;
                let parts: Vec<&str> = line.trim().split(output_separator).collect();

                if parts.len() <= fields.len() { return None; }

                let job = Self::parse_job_from_parts(
                    &parts,
                    (parts[13], parts[14], parts[0], parts[15], parts[3], parts[1], parts[16]),
                );
                Some(job)
            })
            .collect();

        // **THE CRITICAL FIX IS HERE**
        // If stdout was not empty, but we couldn't parse any jobs, it means the
        // --Format flag was likely ignored. This is a failure, so we must return an error
        // to trigger the fallback to the legacy method.
        if jobs.is_empty() && !output.stdout.is_empty() {
            return Err("Modern squeue produced output, but no jobs could be parsed.".into());
        }

        Ok(jobs)
    }

    /// Fetches jobs using the legacy `squeue -o` method.
    fn fetch_and_parse_legacy(&self) -> Result<Vec<Job>, Box<dyn Error>> {
        const DELIMITER: &str = "|";
        let fields = [
            "%i", "%j", "%T", "%u", "%M", "%b", "%P", "%N", "%o", "%e", "N/A", "%t", "%r", "%A", "%a", "%Z",
        ];
        let output_format = fields.join(DELIMITER);

        let output = Command::new("squeue")
            .args(&self.squeue_args)
            .arg("--array")
            .arg("--noheader")
            .arg("-o")
            .arg(&output_format)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Legacy squeue command failed. Stderr: {}", stderr).into());
        }
        
        let jobs = output
            .stdout
            .lines()
            .filter_map(|l| {
                let line = l.ok()?;
                let parts: Vec<&str> = line.trim().split(DELIMITER).collect();

                if parts.len() != fields.len() { return None; }

                let job = Self::parse_job_from_parts(
                    &parts,
                    (parts[13], parts[14], parts[0], parts[7], parts[3], parts[1], parts[15]),
                );
                Some(job)
            })
            .collect();

        Ok(jobs)
    }

    /// A helper function to create a Job struct from a slice of string parts.
    fn parse_job_from_parts(parts: &[&str], path_info: (&str, &str, &str, &str, &str, &str, &str)) -> Job {
        let (array_job_id, array_task_id, id, nodelist, user, name, working_dir) = path_info;

        Job {
            job_id: parts[0].to_owned(),
            name: parts[1].to_owned(),
            state: parts[2].to_owned(),
            user: parts[3].to_owned(),
            time: parts[4].to_owned(),
            tres: parts[5].to_owned(),
            partition: parts[6].to_owned(),
            nodelist: parts[7].to_owned(),
            stdout: Self::resolve_path(parts[8], array_job_id, array_task_id, id, nodelist, user, name, working_dir),
            stderr: Self::resolve_path(parts[9], array_job_id, array_task_id, id, nodelist, user, name, working_dir),
            command: parts[10].to_owned(),
            state_compact: parts[11].to_owned(),
            reason: if parts[12] == "None" || parts[12].is_empty() { None } else { Some(parts[12].to_owned()) },
            array_id: parts[13].to_owned(),
            array_step: if parts[14] == "N/A" { None } else { Some(parts[14].to_owned()) },
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
        lazy_static::lazy_static! {
            static ref RE: Regex = Regex::new(r"%(%|A|a|J|j|N|n|s|t|u|x)").unwrap();
        }

        let mut path = path.trim().to_owned();
        let slurm_no_val = "4294967294";
        let effective_array_id = if array_id == "N/A" { slurm_no_val } else { array_id };

        if path.is_empty() || path == "(null)" {
            path = if effective_array_id == slurm_no_val {
                format!("slurm-{}.out", id)
            } else {
                format!("slurm-{}_{}.out", array_master, effective_array_id)
            };
        }

        for cap in RE.captures_iter(&path.clone()).collect::<Vec<_>>().iter().rev() {
            let m = cap.get(0).unwrap();
            let replacement = match m.as_str() {
                "%%" => "%",
                "%A" => array_master,
                "%a" => effective_array_id,
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

        let mut path_buf = PathBuf::from(path);
        if path_buf.is_relative() && !working_dir.is_empty() {
            path_buf = PathBuf::from(working_dir).join(path_buf);
        }

        Some(path_buf)
    }
}

impl JobWatcherHandle {
    pub fn new(app: Sender<AppMessage>, interval: Duration, squeue_args: Vec<String>) -> Self {
        let mut actor = JobWatcher::new(app, interval, squeue_args);
        thread::spawn(move || actor.run());
        Self {}
    }
}