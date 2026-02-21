use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use rayon::prelude::*;

#[derive(Parser)]
#[command(arg_required_else_help = true)]
struct Args {
    /// Script file to run
    file: Option<PathBuf>,

    /// Read commands from stdin instead of a file
    #[arg(long, conflicts_with = "file")]
    stdin: bool,

    /// Command to run; may be specified multiple times
    #[arg(long = "command", short = 'c')]
    commands: Vec<String>,

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

    /// Number of parallel jobs; defaults to 2x CPU thread count
    #[arg(long, short = 'j')]
    jobs: Option<usize>,
}

fn make_label(cmd: &str, label_width: usize) -> String {
    let label_width = label_width.max(10);
    if cmd.chars().count() > label_width {
        let truncated: String = cmd.chars().take(label_width - 3).collect();
        format!("{}...", truncated)
    } else {
        cmd.to_string()
    }
}

fn run_command(
    cmd: &str,
    shell: &str,
    shell_args: &[String],
    label: &str,
    max_len: usize,
) -> Result<bool> {
    let mut child = Command::new(shell)
        .args(shell_args.iter().chain([&cmd.to_string()]))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn process")?;

    let stdout = child.stdout.take().context("failed to capture stdout")?;
    let stderr = child.stderr.take().context("failed to capture stderr")?;

    let label_err = label.to_string();
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
    Ok(status.success())
}

fn run_commands(
    tasks: Vec<String>,
    shell: String,
    shell_args: Vec<String>,
    label_width: usize,
    pool: &rayon::ThreadPool,
) -> Result<Vec<Option<String>>> {
    let max_len = tasks
        .iter()
        .map(|t| t.chars().count())
        .max()
        .unwrap_or(0)
        .min(label_width);

    pool.install(|| {
        tasks
            .into_par_iter()
            .map(|cmd| -> Result<Option<String>> {
                let label = make_label(&cmd, label_width);
                match run_command(&cmd, &shell, &shell_args, &label, max_len) {
                    Ok(true) => Ok(None),
                    Ok(false) => Ok(Some(cmd)),
                    Err(e) => {
                        println!("{label:<max_len$} ! {e:#}");
                        Ok(Some(cmd))
                    }
                }
            })
            .collect()
    })
}

fn parse_tasks(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
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

    let mut tasks = Vec::new();

    if let Some(path) = args.file {
        let file = std::fs::File::open(&path).context("failed to open script file")?;
        let lines = BufReader::new(file)
            .lines()
            .map(|l| l.context("failed to read line"))
            .collect::<Result<Vec<String>>>()?;
        tasks.extend(parse_tasks(lines));
    }

    if args.stdin {
        let stdin = std::io::stdin();
        let lines = stdin
            .lock()
            .lines()
            .map(|l| l.context("failed to read stdin"))
            .collect::<Result<Vec<String>>>()?;
        tasks.extend(parse_tasks(lines));
    }

    tasks.extend(args.commands);

    let jobs = args.jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            * 2
    });
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("failed to build thread pool")?;

    let failed: Vec<String> =
        run_commands(tasks, args.shell, args.shell_args, args.label_width, &pool)?
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
        std::process::exit(failed.len().min(20) as i32);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    mod parse_tasks_tests {
        use super::*;

        #[test]
        fn filters_empty_lines() {
            assert_eq!(
                parse_tasks(strs(&["cmd1", "", "cmd2"])),
                strs(&["cmd1", "cmd2"])
            );
        }

        #[test]
        fn filters_comment_lines() {
            assert_eq!(
                parse_tasks(strs(&["# comment", "cmd", "# another"])),
                strs(&["cmd"])
            );
        }

        #[test]
        fn trims_whitespace() {
            assert_eq!(
                parse_tasks(strs(&["  cmd  ", "\tcmd2\t"])),
                strs(&["cmd", "cmd2"])
            );
        }

        #[test]
        fn trims_then_filters_blank() {
            assert_eq!(parse_tasks(strs(&["   ", "\t", "cmd"])), strs(&["cmd"]));
        }

        #[test]
        fn trims_then_filters_indented_comment() {
            assert_eq!(
                parse_tasks(strs(&["  # indented comment", "cmd"])),
                strs(&["cmd"])
            );
        }

        #[test]
        fn empty_input_returns_empty() {
            assert!(parse_tasks(vec![]).is_empty());
        }

        #[test]
        fn all_filtered_returns_empty() {
            assert!(parse_tasks(strs(&["", "# comment", "   "])).is_empty());
        }

        #[test]
        fn preserves_order() {
            assert_eq!(parse_tasks(strs(&["a", "b", "c"])), strs(&["a", "b", "c"]));
        }

        #[test]
        fn shebang_line_treated_as_comment() {
            assert_eq!(
                parse_tasks(strs(&["#!/usr/bin/env waffle --shebang", "cmd"])),
                strs(&["cmd"])
            );
        }
    }

    mod make_label_tests {
        use super::*;

        #[test]
        fn short_command_unchanged() {
            assert_eq!(make_label("echo hello", 32), "echo hello");
        }

        #[test]
        fn exact_width_unchanged() {
            let cmd = "a".repeat(32);
            assert_eq!(make_label(&cmd, 32), cmd);
        }

        #[test]
        fn longer_than_width_truncated_with_ellipsis() {
            let cmd = "a".repeat(40);
            let result = make_label(&cmd, 32);
            assert_eq!(result, format!("{}...", "a".repeat(29)));
            assert_eq!(result.chars().count(), 32);
        }

        #[test]
        fn truncated_label_has_correct_length() {
            let result = make_label("this is a very long command that exceeds the limit", 20);
            assert!(result.ends_with("..."));
            assert_eq!(result.chars().count(), 20);
        }

        #[test]
        fn unicode_chars_counted_correctly() {
            // 15 emoji chars, limit 10 → take 7 + "..."
            let cmd = "😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀".repeat(15);
            let result = make_label(&cmd, 10);
            assert_eq!(result, "😀😀😀😀😀😀😀...");
            assert_eq!(result.chars().count(), 10);
        }

        #[test]
        fn small_width_coerced_to_10() {
            for raw in [0usize, 1, 2, 3, 9] {
                let result = make_label(&"aaaaaaaaaaaaaaaaaaaa", raw);
                assert_eq!(result.chars().count(), 10);
                assert!(result.ends_with("..."));
            }
        }
    }
}
