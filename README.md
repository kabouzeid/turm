# turm

![PyPI - Version](https://img.shields.io/pypi/v/turm)
![Crates.io Version](https://img.shields.io/crates/v/turm)

A text-based user interface (TUI) for the [Slurm Workload Manager](https://slurm.schedmd.com/), which provides a convenient way to manage your cluster jobs.

<img alt="turm demo" src="https://user-images.githubusercontent.com/7303830/228503846-3e5abc04-2c1e-422e-844b-d12ca097403a.gif" width="100%" />

`turm` accepts the same options as `squeue` (see [man squeue](https://slurm.schedmd.com/squeue.html#SECTION_OPTIONS)). Use `turm --help` to get a list of all available options.

## Installation

`turm` is available on [PyPI](https://pypi.org/project/turm/) and [crates.io](https://crates.io/crates/turm):

```shell
# With uv.
uv tool install turm

# With pip.
pip install turm

# With cargo.
cargo install turm

# With wget. Make sure ~/.local/bin is in your $PATH.
wget https://github.com/kabouzeid/turm/releases/latest/download/turm-x86_64-unknown-linux-musl.tar.gz -O - | tar -xz -C ~/.local/bin/
```

The [release page](https://github.com/kabouzeid/turm/releases) also contains precompiled binaries for Linux.

### Shell Completion (optional)

#### Bash

In your `.bashrc`, add the following line:
```bash
eval "$(turm completion bash)"
```

#### Zsh

In your `.zshrc`, add the following line:
```zsh
eval "$(turm completion zsh)"
```

#### Fish

In your `config.fish` or in a separate `completions/turm.fish` file, add the following line:
```fish
turm completion fish | source
```

## How it works

`turm` obtains information about jobs by parsing the output of `squeue`.
The reason for this is that `squeue` is available on all Slurm clusters, and running it periodically is not too expensive for the Slurm controller ( particularly when [filtering by user](https://slurm.schedmd.com/squeue.html#OPT_user)).
In contrast, Slurm's C API is unstable, and Slurm's REST API is not always available and can be costly for the Slurm controller.
Another advantage is that we get free support for the exact same CLI flags as `squeue`, which users are already familiar with, for filtering and sorting the jobs.

### Ressource usage

TL;DR: `turm` ≈ `watch -n2 squeue` + `tail -f slurm-log.out`

Special care has been taken to ensure that `turm` is as lightweight as possible in terms of its impact on the Slurm controller and its file I/O operations.
The job queue is updated every two seconds by running `squeue`.
When there are many jobs in the queue, it is advisable to specify a single user to reduce the load on the Slurm controller (see [squeue --user](https://slurm.schedmd.com/squeue.html#OPT_user)).
`turm` updates the currently displayed log file on every inotify modify notification, and it only reads the newly appended lines after the initial read.
However, since inotify notifications are not supported for remote file systems, such as NFS, `turm` also polls the file for newly appended bytes every two seconds.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=kabouzeid/turm&type=Date)](https://www.star-history.com/#kabouzeid/turm&Date)
