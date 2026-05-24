use std::io::{BufRead, BufReader, Write};
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

    /// Read commands from stdin
    #[arg(long)]
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

    /// Suppress per-command output; only print output of failed commands
    #[arg(long, short = 'q')]
    quiet: bool,

    /// Output destination for command logs. Use "-" for stdout (default).
    /// A file path may contain {cmd}, {order}, and {timestamp} placeholders.
    /// {order} is the 1-based position of the command in the input.
    /// With --quiet, only failed commands produce output.
    /// When output goes to a file, the path is printed to stdout.
    #[arg(long, short = 'o', default_value = "-")]
    output: String,

    /// Change working directory to the directory containing the script file before running commands
    #[arg(long)]
    chdir_script_dir: bool,
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

fn make_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    let (y, mo, d) = {
        let mut y = 1970i64;
        let mut rem = days as i64;
        loop {
            let ydays = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
            if rem < ydays { break; }
            rem -= ydays;
            y += 1;
        }
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let mdays = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut mo = 0;
        for (i, &md) in mdays.iter().enumerate() {
            if rem < md as i64 { mo = i + 1; break; }
            rem -= md as i64;
        }
        (y, mo, (rem + 1) as u64)
    };
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

fn sanitize_for_filename(s: &str) -> String {
    let raw: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut result = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for c in raw.chars() {
        if c == '_' {
            if !prev_underscore {
                result.push('_');
            }
            prev_underscore = true;
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }
    result
}

fn resolve_output_path(pattern: &str, label: &str, order: usize, order_width: usize) -> PathBuf {
    let safe_label = sanitize_for_filename(label);
    PathBuf::from(
        pattern
            .replace("{cmd}", &safe_label)
            .replace("{order}", &format!("{order:0>order_width$}")),
    )
}

fn open_append(path: &std::path::Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open output file: {}", path.display()))
}

fn run_command(
    cmd: &str,
    shell: &str,
    shell_args: &[String],
    label: &str,
    max_len: usize,
    quiet: bool,
    output_path: Option<&std::path::Path>,
    cwd: Option<&std::path::Path>,
) -> Result<bool> {
    let mut builder = Command::new(shell);
    builder
        .args(shell_args.iter().chain([&cmd.to_string()]))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        builder.current_dir(dir);
    }
    let mut child = builder.spawn().context("failed to spawn process")?;

    let stdout = child.stdout.take().context("failed to capture stdout")?;
    let stderr = child.stderr.take().context("failed to capture stderr")?;

    let is_file = output_path.is_some();

    let label_err = label.to_string();
    let stderr_quiet = quiet;
    let stderr_is_file = is_file;
    let stderr_thread = thread::spawn(move || -> Result<Vec<String>> {
        let mut buf = Vec::new();
        for line in BufReader::new(stderr).lines() {
            let line = line.context("failed to read stderr line")?;
            if stderr_quiet || stderr_is_file {
                buf.push(line);
            } else {
                println!("{label_err:<max_len$} ! {line}");
            }
        }
        Ok(buf)
    });

    let mut stdout_buf = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.context("failed to read stdout line")?;
        if quiet || is_file {
            stdout_buf.push(line);
        } else {
            println!("{label:<max_len$} | {line}");
        }
    }

    let stderr_buf = stderr_thread
        .join()
        .map_err(|_| anyhow!("stderr thread panicked"))??;

    let status = child.wait().context("child process failed")?;

    let should_write = if quiet { !status.success() } else { true };

    if let Some(path) = output_path {
        if should_write {
            let mut buf = String::new();
            for line in &stdout_buf {
                use std::fmt::Write as _;
                let _ = writeln!(buf, "{label:<max_len$} | {line}");
            }
            for line in &stderr_buf {
                use std::fmt::Write as _;
                let _ = writeln!(buf, "{label:<max_len$} ! {line}");
            }
            let mut file = open_append(path)?;
            file.write_all(buf.as_bytes())
                .with_context(|| format!("failed to write to output file: {}", path.display()))?;
        }
    } else if quiet && !status.success() {
        for line in &stdout_buf {
            println!("{label:<max_len$} | {line}");
        }
        for line in &stderr_buf {
            println!("{label:<max_len$} ! {line}");
        }
    }

    Ok(status.success())
}

fn run_commands(
    tasks: Vec<String>,
    shell: String,
    shell_args: Vec<String>,
    label_width: usize,
    quiet: bool,
    output_pattern: Option<&str>,
    cwd: Option<&std::path::Path>,
    pool: &rayon::ThreadPool,
) -> Result<Vec<Option<String>>> {
    let max_len = tasks
        .iter()
        .map(|t| t.chars().count())
        .max()
        .unwrap_or(0)
        .min(label_width);

    let order_width = tasks.len().to_string().len();
    let output_paths: Option<Vec<PathBuf>> = output_pattern.map(|pat| {
        tasks
            .iter()
            .enumerate()
            .map(|(i, cmd)| resolve_output_path(pat, &make_label(cmd, label_width), i + 1, order_width))
            .collect()
    });

    pool.install(|| {
        tasks
            .into_par_iter()
            .enumerate()
            .map(|(i, cmd)| -> Result<Option<String>> {
                let label = make_label(&cmd, label_width);
                let path = output_paths.as_ref().map(|paths| paths[i].as_path());
                match run_command(&cmd, &shell, &shell_args, &label, max_len, quiet, path, cwd) {
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
    let mut script_dir: Option<PathBuf> = None;

    if args.chdir_script_dir && args.file.is_none() {
        eprintln!("warning: --chdir-script-dir has no effect without a script file");
    }

    if let Some(path) = args.file {
        if args.chdir_script_dir {
            script_dir = path
                .canonicalize()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()));
            if script_dir.is_none() {
                eprintln!("warning: --chdir-here could not resolve the script's directory");
            }
        }
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

    let output_pattern = if args.output == "-" {
        None
    } else {
        Some(args.output.replace("{timestamp}", &make_timestamp()))
    };

    if args.quiet {
        println!("running...");
    }

    let all_tasks = tasks.clone();

    let failed: Vec<String> =
        run_commands(tasks, args.shell, args.shell_args, args.label_width, args.quiet, output_pattern.as_deref(), script_dir.as_deref(), &pool)?
            .into_iter()
            .flatten()
            .collect();

    if let Some(pat) = &output_pattern {
        let mut printed = std::collections::BTreeSet::new();
        let written_cmds: Vec<_> = if args.quiet {
            failed
                .iter()
                .filter_map(|cmd| {
                    all_tasks
                        .iter()
                        .position(|t| t == cmd)
                        .map(|i| (i + 1, cmd))
                })
                .collect()
        } else {
            all_tasks.iter().enumerate().map(|(i, cmd)| (i + 1, cmd)).collect()
        };
        let order_width = all_tasks.len().to_string().len();
        for (order, cmd) in written_cmds {
            let label = make_label(cmd, args.label_width);
            let path = resolve_output_path(pat, &label, order, order_width);
            if path.exists() {
                printed.insert(path);
            }
        }
        for path in &printed {
            println!("{}", path.display());
        }
    }

    if !args.skip_report_failures && !failed.is_empty() {
        println!("\nfailed:");
        for cmd in &failed {
            println!("  {cmd}");
        }
    }

    if failed.is_empty() && args.quiet {
        println!("all commands succeeded");
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

    mod sanitize_for_filename_tests {
        use super::*;

        #[test]
        fn alphanumeric_unchanged() {
            assert_eq!(sanitize_for_filename("echo-hello_world"), "echo-hello_world");
        }

        #[test]
        fn dots_replaced() {
            assert_eq!(sanitize_for_filename("file.txt"), "file_txt");
        }

        #[test]
        fn spaces_become_underscore() {
            assert_eq!(sanitize_for_filename("echo hello"), "echo_hello");
        }

        #[test]
        fn slashes_become_underscore() {
            assert_eq!(sanitize_for_filename("ls /tmp/foo"), "ls_tmp_foo");
        }

        #[test]
        fn consecutive_special_chars_collapsed() {
            assert_eq!(sanitize_for_filename("a  /  b"), "a_b");
        }

        #[test]
        fn pipes_and_semicolons() {
            assert_eq!(sanitize_for_filename("cat foo | grep bar; wc"), "cat_foo_grep_bar_wc");
        }

        #[test]
        fn quotes_and_parens() {
            assert_eq!(sanitize_for_filename("echo 'hello' \"world\" (test)"), "echo_hello_world_test_");
        }
    }
}
