# turm

A text-based user interface (TUI) for the [Slurm Workload Manager](https://slurm.schedmd.com/), which provides a convenient way to manage your cluster jobs.

## Usage

`turm` accepts the same CLI flags as `squeue`, and passes them to `squeue` when querying the jobs.
```bash
turm [--user <username>] [--partition <partition>] [...]
```

## How it works

`turm` obtains information about jobs by parsing the output of `squeue`.
The reason for this is that `squeue` is available on all Slurm clusters, and running it periodically is not too expensive for the Slurm controller ( particularly when [filtering by user](https://slurm.schedmd.com/squeue.html#OPT_user)).
In contrast, Slurm's C API is unstable, and Slurm's REST API is not always available and can be costly for the Slurm controller.
Another advantage is that we get free support for the exact same CLI flags as `squeue`, which users are already familiar with, for filtering and sorting the jobs.

## Ressource usage

TL;DR: `turm` $\approx$ `watch -n2 squeue` + `tail -f slurm-log.out`

Special care has been taken to ensure that `turm` is as lightweight as possible in terms of its impact on the Slurm controller and its file I/O operations.
The job queue is updated every two seconds by running `squeue`.
When there are many jobs in the queue, it is advisable to specify a single user to reduce the load on the Slurm controller (see [squeue --user](https://slurm.schedmd.com/squeue.html#OPT_user)).
`turm` updates the currently displayed log file on every inotify modify notification, and it only reads the newly appended lines after the initial read.
However, since inotify notifications are not supported for remote file systems, such as NFS, `turm` also polls the file for newly appended bytes every two seconds.
