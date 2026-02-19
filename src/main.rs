use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};

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
}

fn run_commands(tasks: Vec<String>) -> Vec<JoinHandle<Option<String>>> {
    tasks
        .into_iter()
        .map(|cmd| {
            thread::spawn(move || {
                let mut child = Command::new("sh")
                    .args(["-c", &cmd])
                    .stdout(Stdio::piped())
                    .spawn()
                    .expect("failed to spawn process");

                let stdout = child.stdout.take().expect("failed to capture stdout");
                let reader = BufReader::new(stdout);

                for line in reader.lines() {
                    let line = line.expect("failed to read line");
                    println!("[{cmd}]: {line}");
                }

                let status = child.wait().expect("child process failed");
                if status.success() { None } else { Some(cmd) }
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

fn main() {
    let args = Args::parse();

    let tasks = if args.mode.stdin {
        let stdin = std::io::stdin();
        parse_tasks(
            stdin
                .lock()
                .lines()
                .map(|l| l.expect("failed to read stdin")),
        )
    } else {
        let path = args
            .file
            .expect("a script file is required (or use --stdin)");
        let file = std::fs::File::open(&path).expect("failed to open script file");
        let lines = BufReader::new(file)
            .lines()
            .map(|l| l.expect("failed to read line"));
        parse_tasks(lines)
    };

    let failed: Vec<String> = run_commands(tasks)
        .into_iter()
        .filter_map(|h| h.join().expect("thread panicked"))
        .collect();

    if !failed.is_empty() {
        println!("\nfailed:");
        for name in failed {
            println!("  {name}");
        }
    }
}
