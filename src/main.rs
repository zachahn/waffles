use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};

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

fn main() {
    let stdin = std::io::stdin();
    let tasks: Vec<String> = stdin
        .lock()
        .lines()
        .filter_map(|line| {
            let line = line.expect("failed to read stdin");
            let line = line.trim().to_string();
            if line.is_empty() { None } else { Some(line) }
        })
        .collect();

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
