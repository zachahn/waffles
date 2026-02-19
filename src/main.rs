use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::thread;

fn main() {
    let stdin = std::io::stdin();
    let tasks: Vec<(String, String)> = stdin
        .lock()
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.expect("failed to read stdin");
            let line = line.trim().to_string();
            if line.is_empty() {
                return None;
            }
            if let Some((name, cmd)) = line.split_once(':') {
                Some((name.trim().to_string(), cmd.trim().to_string()))
            } else {
                Some(((i + 1).to_string(), line))
            }
        })
        .collect();

    let handles: Vec<_> = tasks
        .into_iter()
        .map(|(name, cmd)| {
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
                    println!("[{name}]: {line}");
                }

                child.wait().expect("child process failed");
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("thread panicked");
    }
}
