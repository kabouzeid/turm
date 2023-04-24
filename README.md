# turm

A text-based user interface (TUI) for the [Slurm Workload Manager](https://slurm.schedmd.com/), which provides a convenient way to manage your cluster jobs.

<img alt="turm demo" src="https://user-images.githubusercontent.com/7303830/228503846-3e5abc04-2c1e-422e-844b-d12ca097403a.gif" width="100%" />

`turm` accepts the same options as `squeue` (see [man squeue](https://slurm.schedmd.com/squeue.html#SECTION_OPTIONS)). Use `turm --help` to get a list of all available options.

## Project status

This project is currently a work in progress but the basic functionality is there.
There are still some missing features that need to be implemented and a few visual bugs present at the moment. 
Please feel free to submit any issues or feedback you may have.

## Installation

### From source

```bash
cargo install turm
```

### From binaries

The [release page](https://github.com/kabouzeid/turm/releases) includes precompiled binaries for Linux, macOS and Windows.
Statically-linked binaries are also available: look for archives with `musl` in the file name.

## Shell Completion

### Bash

In your `.bashrc`, add the following line:
```bash
eval "$(turm completion bash)"
```

### Zsh

In your `.zshrc`, add the following line:
```zsh
eval "$(turm completion zsh)"
```

### Other Shells

Completion scripts for other shells (`fish`, `elvish` and `powershell`) can be generated with `turm completion <shell>`.

## How it works

`turm` obtains information about jobs by parsing the output of `squeue`.
The reason for this is that `squeue` is available on all Slurm clusters, and running it periodically is not too expensive for the Slurm controller ( particularly when [filtering by user](https://slurm.schedmd.com/squeue.html#OPT_user)).
In contrast, Slurm's C API is unstable, and Slurm's REST API is not always available and can be costly for the Slurm controller.
Another advantage is that we get free support for the exact same CLI flags as `squeue`, which users are already familiar with, for filtering and sorting the jobs.

## Ressource usage

TL;DR: `turm` ≈ `watch -n2 squeue` + `tail -f slurm-log.out`

Special care has been taken to ensure that `turm` is as lightweight as possible in terms of its impact on the Slurm controller and its file I/O operations.
The job queue is updated every two seconds by running `squeue`.
When there are many jobs in the queue, it is advisable to specify a single user to reduce the load on the Slurm controller (see [squeue --user](https://slurm.schedmd.com/squeue.html#OPT_user)).
`turm` updates the currently displayed log file on every inotify modify notification, and it only reads the newly appended lines after the initial read.
However, since inotify notifications are not supported for remote file systems, such as NFS, `turm` also polls the file for newly appended bytes every two seconds.
