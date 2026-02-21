use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn waffle() -> Command {
    Command::new(env!("CARGO_BIN_EXE_waffles"))
}

/// Temporary file that deletes itself on drop.
struct TempScript(PathBuf);

impl TempScript {
    fn new(content: &str) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        // Include thread id for parallel-test safety.
        let tid = std::thread::current().id();
        let path = std::env::temp_dir().join(format!("waffle_test_{ns}_{tid:?}.sh"));
        std::fs::write(&path, content).unwrap();
        TempScript(path)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Run waffle with commands piped via --stdin. Returns (stdout, stderr, exit_code).
fn run_stdin(cmds: &str, extra_args: &[&str]) -> (String, String, i32) {
    let mut child = waffle()
        .arg("--stdin")
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn waffle");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(cmds.as_bytes())
        .unwrap();

    let out = child.wait_with_output().expect("failed to wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Run waffle with a temporary script file. Returns (stdout, stderr, exit_code).
fn run_file(cmds: &str, extra_args: &[&str]) -> (String, String, i32) {
    let script = TempScript::new(cmds);

    let out = waffle()
        .arg(script.path())
        .args(extra_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run waffle");

    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

// ── exit codes ──────────────────────────────────────────────────────────────

#[test]
fn all_succeed_exits_zero() {
    let (_, _, code) = run_stdin("echo hello\necho world\n", &[]);
    assert_eq!(code, 0);
}

#[test]
fn one_failure_exits_nonzero() {
    let (_, _, code) = run_stdin("false\n", &[]);
    assert_ne!(code, 0);
}

#[test]
fn exit_code_equals_number_of_failures() {
    let (_, _, code) = run_stdin("false\nfalse\nfalse\n", &[]);
    assert_eq!(code, 3);
}

#[test]
fn exit_code_capped_at_twenty() {
    let cmds: String = "false\n".repeat(25);
    let (_, _, code) = run_stdin(&cmds, &[]);
    assert_eq!(code, 20);
}

#[test]
fn mixed_success_and_failure_counts_only_failures() {
    let (_, _, code) = run_stdin("true\nfalse\ntrue\nfalse\n", &[]);
    assert_eq!(code, 2);
}

// ── output format ────────────────────────────────────────────────────────────

#[test]
fn stdout_lines_contain_pipe_separator() {
    let (out, _, _) = run_stdin("echo hello\n", &[]);
    assert!(out.contains("| hello"), "expected '| hello' in: {out:?}");
}

#[test]
fn stderr_lines_contain_bang_separator() {
    let (out, _, _) = run_stdin("echo err >&2\n", &[]);
    assert!(out.contains("! err"), "expected '! err' in: {out:?}");
}

#[test]
fn label_is_the_command() {
    let (out, _, _) = run_stdin("echo hi\n", &[]);
    assert!(out.contains("echo hi"), "expected label in: {out:?}");
}

// ── failure report ───────────────────────────────────────────────────────────

#[test]
fn failed_section_printed_on_failure() {
    let (out, _, _) = run_stdin("false\n", &[]);
    assert!(out.contains("failed:"), "expected 'failed:' in: {out:?}");
    assert!(out.contains("false"), "expected failed cmd in: {out:?}");
}

#[test]
fn failed_section_suppressed_with_flag() {
    let (out, _, code) = run_stdin("false\n", &["--skip-report-failures"]);
    assert!(!out.contains("failed:"), "unexpected 'failed:' in: {out:?}");
    assert_ne!(code, 0);
}

#[test]
fn no_failed_section_when_all_succeed() {
    let (out, _, _) = run_stdin("true\n", &[]);
    assert!(!out.contains("failed:"), "unexpected 'failed:' in: {out:?}");
}

// ── label truncation ─────────────────────────────────────────────────────────

#[test]
fn long_label_truncated_to_label_width() {
    let long_cmd = format!("echo {}", "x".repeat(50));
    let (out, _, _) = run_stdin(&format!("{long_cmd}\n"), &["--label-width", "20"]);
    // truncated label ends with "..." and should appear in output
    assert!(out.contains("..."), "expected truncated label in: {out:?}");
}

#[test]
fn short_label_not_truncated() {
    let (out, _, _) = run_stdin("echo hi\n", &["--label-width", "32"]);
    assert!(!out.contains("..."), "unexpected ellipsis in: {out:?}");
}

// ── input modes ──────────────────────────────────────────────────────────────

#[test]
fn file_mode_runs_commands() {
    let (out, _, code) = run_file("echo fromfile\n", &[]);
    assert_eq!(code, 0);
    assert!(out.contains("fromfile"), "expected output in: {out:?}");
}

#[test]
fn file_mode_comments_and_blanks_ignored() {
    // only "echo ok" should run; if the comment ran it would fail or produce unexpected output
    let (out, _, code) = run_file("# this is a comment\n\necho ok\n", &[]);
    assert_eq!(code, 0);
    assert!(out.contains("ok"));
    assert!(!out.contains("this is a comment"));
}

#[test]
fn stdin_mode_empty_input_exits_zero() {
    let (_, _, code) = run_stdin("", &[]);
    assert_eq!(code, 0);
}

#[test]
fn stdin_mode_comments_and_blanks_ignored() {
    let (out, _, code) = run_stdin("# comment\n\necho real\n", &[]);
    assert_eq!(code, 0);
    assert!(out.contains("real"));
}

// ── shebang ───────────────────────────────────────────────────────────────────

#[test]
fn shebang_line_treated_as_comment() {
    // The shebang line starts with '#' so parse_tasks filters it; it must not appear as a label
    let (out, _, code) = run_file("#!/usr/bin/env waffle\necho hello\n", &[]);
    assert_eq!(code, 0, "stdout: {out}");
    assert!(out.contains("hello"), "expected 'hello' in: {out:?}");
    assert!(
        !out.contains("#!/usr/bin/env"),
        "shebang line should not run"
    );
}

// ── custom shell ─────────────────────────────────────────────────────────────

#[test]
fn custom_shell_used() {
    // Use bash explicitly; it's available on macOS and Linux
    let (out, _, code) = run_stdin("echo bash_ran\n", &["--shell", "/bin/bash"]);
    assert_eq!(code, 0);
    assert!(out.contains("bash_ran"), "expected output in: {out:?}");
}

#[test]
fn bad_shell_path_all_commands_fail() {
    // /yolo/bin does not exist; every spawn should fail and be reported
    let cmds = "echo 1 && exit 1\necho 2 && exit 2\necho 3 && exit 3\necho 4 && exit 4\n";
    let (out, _, code) = run_stdin(cmds, &["--shell", "/fail"]);
    // all 4 commands fail to spawn → exit code 4
    assert_eq!(code, 4, "stdout: {out}");
    // each failure should produce an error line with '!'
    assert!(out.contains('!'), "expected error output in: {out:?}");
    // failure summary should list commands
    assert!(out.contains("failed:"), "expected 'failed:' in: {out:?}");
}
