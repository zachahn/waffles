use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, anyhow};
use clap::Parser;

#[derive(clap::Args)]
struct ModeArgs {
    /// Read commands from stdin instead of a file
    #[arg(long, conflicts_with = "file")]
    stdin: bool,

    /// Run as a shebang interpreter (add to top of your script: #!/usr/bin/env waffle --shebang)
    #[arg(long, requires = "file")]
    shebang: bool,
}

#[derive(Parser)]
#[command(arg_required_else_help = true)]
struct Args {
    /// Script file to run
    file: Option<PathBuf>,

    #[command(flatten)]
    mode: ModeArgs,

    /// Shell to use for running commands
    #[arg(long, default_value = "/bin/sh")]
    shell: String,

    /// Argument to pass to the shell before the command; may be specified multiple times
    #[arg(long = "shell-arg", default_values = ["-c"])]
    shell_args: Vec<String>,

    /// Do not print a summary of failed commands at the end
    #[arg(long)]
    skip_report_failures: bool,

    /// Maximum width for the command label column; longer commands are truncated
    #[arg(long, default_value_t = 32)]
    label_width: usize,
}

fn run_commands(
    tasks: Vec<String>,
    shell: String,
    shell_args: Vec<String>,
    label_width: usize,
) -> Vec<JoinHandle<Result<Option<String>>>> {
    let max_len = tasks
        .iter()
        .map(|t| t.len())
        .max()
        .unwrap_or(0)
        .min(label_width);
    tasks
        .into_iter()
        .map(|cmd| {
            let shell = shell.clone();
            let shell_args = shell_args.clone();
            thread::spawn(move || -> Result<Option<String>> {
                let label = if cmd.len() > label_width {
                    format!("{}...", &cmd[..label_width - 3])
                } else {
                    cmd.clone()
                };

                let mut child = Command::new(&shell)
                    .args(shell_args.iter().chain([&cmd]))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn process")?;

                let stdout = child.stdout.take().context("failed to capture stdout")?;
                let stderr = child.stderr.take().context("failed to capture stderr")?;

                let label_err = label.clone();
                let stderr_thread = thread::spawn(move || -> Result<()> {
                    for line in BufReader::new(stderr).lines() {
                        let line = line.context("failed to read stderr line")?;
                        println!("{label_err:<max_len$} ! {line}");
                    }
                    Ok(())
                });

                for line in BufReader::new(stdout).lines() {
                    let line = line.context("failed to read stdout line")?;
                    println!("{label:<max_len$} | {line}");
                }

                stderr_thread
                    .join()
                    .map_err(|_| anyhow!("stderr thread panicked"))??;

                let status = child.wait().context("child process failed")?;
                Ok(if status.success() { None } else { Some(cmd) })
            })
        })
        .collect()
}

fn parse_tasks(lines: impl Iterator<Item = String>) -> Vec<String> {
    lines
        .filter_map(|line| {
            let line = line.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                None
            } else {
                Some(line)
            }
        })
        .collect()
}

fn main() -> Result<()> {
    let args = Args::parse();

    let tasks = if args.mode.stdin {
        let stdin = std::io::stdin();
        let lines = stdin
            .lock()
            .lines()
            .map(|l| l.context("failed to read stdin"))
            .collect::<Result<Vec<String>>>()?;
        parse_tasks(lines.into_iter())
    } else {
        let path = args
            .file
            .context("a script file is required (or use --stdin)")?;
        let file = std::fs::File::open(&path).context("failed to open script file")?;
        let lines = BufReader::new(file)
            .lines()
            .map(|l| l.context("failed to read line"))
            .collect::<Result<Vec<String>>>()?;
        parse_tasks(lines.into_iter())
    };

    let failed: Vec<String> = run_commands(tasks, args.shell, args.shell_args, args.label_width)
        .into_iter()
        .map(|h| {
            h.join()
                .map_err(|_| anyhow!("thread panicked"))
                .and_then(|r| r)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();

    if !args.skip_report_failures && !failed.is_empty() {
        println!("\nfailed:");
        for cmd in &failed {
            println!("  {cmd}");
        }
    }

    if !failed.is_empty() {
        std::process::exit(failed.len() as i32);
    }

    Ok(())
}
