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

// ── quiet mode ───────────────────────────────────────────────────────────────

#[test]
fn quiet_prints_running() {
    let (out, _, _) = run_stdin("echo hello\n", &["--quiet"]);
    assert!(out.starts_with("running..."), "expected 'running...' at start: {out:?}");
}

#[test]
fn quiet_all_succeed_prints_success_message() {
    let (out, _, code) = run_stdin("echo hello\necho world\n", &["--quiet"]);
    assert_eq!(code, 0);
    assert!(out.contains("all commands succeeded"), "expected success msg: {out:?}");
}

#[test]
fn quiet_all_succeed_suppresses_command_output() {
    let (out, _, _) = run_stdin("echo hello\n", &["--quiet"]);
    assert!(!out.contains("| hello"), "expected no command output: {out:?}");
}

#[test]
fn quiet_failure_shows_failed_stdout() {
    let (out, _, _) = run_stdin("echo badout && exit 1\n", &["--quiet"]);
    assert!(out.contains("| badout"), "expected failed stdout: {out:?}");
}

#[test]
fn quiet_failure_shows_failed_stderr() {
    let (out, _, _) = run_stdin("echo errmsg >&2 && exit 1\n", &["--quiet"]);
    assert!(out.contains("! errmsg"), "expected failed stderr: {out:?}");
}

#[test]
fn quiet_failure_suppresses_successful_command_output() {
    let (out, _, _) = run_stdin("echo good\necho bad && exit 1\n", &["--quiet"]);
    assert!(!out.contains("| good"), "should not show successful cmd output: {out:?}");
    assert!(out.contains("| bad"), "should show failed cmd output: {out:?}");
}

#[test]
fn quiet_failure_shows_failed_section() {
    let (out, _, _) = run_stdin("false\n", &["--quiet"]);
    assert!(out.contains("failed:"), "expected 'failed:' in: {out:?}");
}

#[test]
fn quiet_failure_no_success_message() {
    let (out, _, _) = run_stdin("false\n", &["--quiet"]);
    assert!(!out.contains("all commands succeeded"), "should not show success msg: {out:?}");
}

#[test]
fn quiet_short_flag_works() {
    let (out, _, code) = run_stdin("echo hi\n", &["-q"]);
    assert_eq!(code, 0);
    assert!(out.contains("all commands succeeded"), "expected success msg: {out:?}");
}

// ── chdir-script-dir ─────────────────────────────────────────────────────────

#[test]
fn chdir_script_dir_sets_cwd_to_script_directory() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let dir = std::env::temp_dir().join(format!("waffle_chdir_test_{ns}"));
    std::fs::create_dir_all(&dir).unwrap();
    let script_path = dir.join("tasks.sh");
    std::fs::write(&script_path, "pwd\n").unwrap();

    let out = waffle()
        .arg(&script_path)
        .arg("--chdir-script-dir")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run waffle");

    let _ = std::fs::remove_dir_all(&dir);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = dir.canonicalize().unwrap_or(dir.clone());
    assert!(
        stdout.contains(expected.to_str().unwrap()),
        "expected {:?} in output: {stdout:?}",
        expected
    );
}

#[test]
fn chdir_script_dir_warning_when_no_file() {
    let (_, stderr, _) = run_stdin("echo hi\n", &["--chdir-script-dir"]);
    assert!(
        stderr.contains("--chdir-script-dir"),
        "expected warning in stderr: {stderr:?}"
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

// ── output option ────────────────────────────────────────────────────────────

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let tid = std::thread::current().id();
        let path = std::env::temp_dir().join(format!("{prefix}_{ns}_{tid:?}"));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn output_dash_is_default_stdout() {
    let (out, _, code) = run_stdin("echo hello\n", &["-o", "-"]);
    assert_eq!(code, 0);
    assert!(out.contains("| hello"), "expected stdout output: {out:?}");
}

#[test]
fn output_file_writes_log() {
    let dir = TempDir::new("waffle_output_test");
    let log_path = dir.path().join("test.log");
    let (out, _, code) = run_stdin("echo fromlog\n", &["-o", log_path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains(log_path.to_str().unwrap()), "expected path printed: {out:?}");
    let content = std::fs::read_to_string(&log_path).expect("log file should exist");
    assert!(content.contains("| fromlog"), "expected log content: {content:?}");
}

#[test]
fn output_file_contains_stderr() {
    let dir = TempDir::new("waffle_output_stderr");
    let log_path = dir.path().join("err.log");
    let (_, _, code) = run_stdin("echo errline >&2\n", &["-o", log_path.to_str().unwrap()]);
    assert_eq!(code, 0);
    let content = std::fs::read_to_string(&log_path).expect("log file should exist");
    assert!(content.contains("! errline"), "expected stderr in log: {content:?}");
}

#[test]
fn output_file_not_written_for_quiet_success() {
    let dir = TempDir::new("waffle_output_quiet_ok");
    let log_path = dir.path().join("quiet_ok.log");
    let (_, _, code) = run_stdin("echo hi\n", &["-q", "-o", log_path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(!log_path.exists(), "log should not exist for quiet success");
}

#[test]
fn output_file_written_for_quiet_failure() {
    let dir = TempDir::new("waffle_output_quiet_fail");
    let log_path = dir.path().join("quiet_fail.log");
    let (out, _, code) = run_stdin("echo failing && exit 1\n", &["-q", "-o", log_path.to_str().unwrap()]);
    assert_ne!(code, 0);
    assert!(out.contains(log_path.to_str().unwrap()), "expected path printed: {out:?}");
    let content = std::fs::read_to_string(&log_path).expect("log file should exist");
    assert!(content.contains("| failing"), "expected failure output in log: {content:?}");
}

#[test]
fn output_name_placeholder_expands() {
    let dir = TempDir::new("waffle_output_cmd");
    let pattern = dir.path().join("{name}.log");
    let (out, _, code) = run_stdin("echo hi\n", &["-o", pattern.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("echo_hi.log"), "expected expanded filename: {out:?}");
}

#[test]
fn output_timestamp_placeholder_expands() {
    let dir = TempDir::new("waffle_output_ts");
    let pattern = dir.path().join("{timestamp}.log");
    let (out, _, code) = run_stdin("echo hi\n", &["-o", pattern.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(!out.contains("{timestamp}"), "timestamp should be expanded: {out:?}");
    let path_line = out.lines().find(|l| l.ends_with(".log")).expect("expected log path line");
    assert!(std::path::Path::new(path_line.trim()).exists(), "expanded log file should exist");
}

#[test]
fn output_shared_file_contains_all_failures() {
    let dir = TempDir::new("waffle_output_shared");
    let log_path = dir.path().join("shared.log");
    let (out, _, code) = run_stdin(
        "echo aaa && exit 1\necho bbb && exit 1\n",
        &["-q", "-o", log_path.to_str().unwrap()],
    );
    assert_ne!(code, 0);
    let content = std::fs::read_to_string(&log_path).expect("shared log should exist");
    assert!(content.contains("aaa"), "expected first failure in log: {content:?}");
    assert!(content.contains("bbb"), "expected second failure in log: {content:?}");
    let path_str = log_path.to_str().unwrap();
    let path_count = out.lines().filter(|l| l.contains(path_str)).count();
    assert_eq!(path_count, 2, "path should appear under each failed command, got {path_count}: {out:?}");
}

#[test]
fn output_shared_file_quiet_success_no_file() {
    let dir = TempDir::new("waffle_output_shared_ok");
    let log_path = dir.path().join("shared_ok.log");
    let (_, _, code) = run_stdin("echo ok1\necho ok2\n", &["-q", "-o", log_path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(!log_path.exists(), "no file for quiet success");
}

#[test]
fn output_order_placeholder_expands() {
    let dir = TempDir::new("waffle_output_order");
    let pattern = dir.path().join("{order}.log");
    let (out, _, code) = run_stdin("echo aaa\necho bbb\n", &["-o", pattern.to_str().unwrap()]);
    assert_eq!(code, 0);
    let p1 = dir.path().join("1.log");
    let p2 = dir.path().join("2.log");
    assert!(p1.exists(), "expected 1.log to exist");
    assert!(p2.exists(), "expected 2.log to exist");
    let c1 = std::fs::read_to_string(&p1).unwrap();
    let c2 = std::fs::read_to_string(&p2).unwrap();
    assert!(c1.contains("aaa"), "expected first command in 1.log: {c1:?}");
    assert!(c2.contains("bbb"), "expected second command in 2.log: {c2:?}");
    assert!(out.contains("1.log"), "expected 1.log in output: {out:?}");
    assert!(out.contains("2.log"), "expected 2.log in output: {out:?}");
}

#[test]
fn output_order_zero_padded_with_ten_or_more_tasks() {
    let dir = TempDir::new("waffle_output_order_pad");
    let pattern = dir.path().join("{order}.log");
    let cmds: Vec<String> = (1..=11).map(|i| format!("echo t{i}")).collect();
    let input = cmds.join("\n") + "\n";
    let (out, _, code) = run_stdin(&input, &["-o", pattern.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(dir.path().join("01.log").exists(), "expected 01.log (zero-padded)");
    assert!(dir.path().join("09.log").exists(), "expected 09.log (zero-padded)");
    assert!(dir.path().join("10.log").exists(), "expected 10.log");
    assert!(dir.path().join("11.log").exists(), "expected 11.log");
    assert!(!dir.path().join("1.log").exists(), "should not have unpadded 1.log");
    assert!(out.contains("01.log"), "expected 01.log in output: {out:?}");
}

#[test]
fn output_cmd_sanitizes_special_chars() {
    let dir = TempDir::new("waffle_output_sanitize");
    let pattern = dir.path().join("{name}.log");
    let (out, _, code) = run_stdin("echo 'hello world'\n", &["-o", pattern.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("echo_hello_world_.log"), "expected sanitized filename: {out:?}");
}

// ── named tasks ────────────────────────────────────────────────────────────

#[test]
fn named_label_is_the_name() {
    let (out, _, code) = run_stdin("greet: echo hello\n", &["--named"]);
    assert_eq!(code, 0);
    assert!(out.contains("greet | hello"), "expected name as label: {out:?}");
    assert!(!out.contains("echo hello |"), "command should not be the label: {out:?}");
}

#[test]
fn named_failure_report_shows_name_and_command() {
    let (out, _, code) = run_stdin("broken: false\n", &["--named"]);
    assert_eq!(code, 1);
    assert!(out.contains("name:    broken"), "expected name in report: {out:?}");
    assert!(out.contains("command: false"), "expected command in report: {out:?}");
}

#[test]
fn named_missing_prefix_errors() {
    let (_, stderr, code) = run_stdin("echo hi\n", &["--named"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("name prefix"), "expected error in stderr: {stderr:?}");
}

#[test]
fn named_uppercase_name_errors() {
    let (_, stderr, code) = run_stdin("Build: echo hi\n", &["--named"]);
    assert_ne!(code, 0);
    assert!(stderr.to_lowercase().contains("name"), "expected name error: {stderr:?}");
}

#[test]
fn named_empty_command_errors() {
    let (_, stderr, code) = run_stdin("build:\n", &["--named"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("missing name prefix"), "expected missing prefix error: {stderr:?}");
}

#[test]
fn named_colon_without_space_errors() {
    let (_, stderr, code) = run_stdin("build:make\n", &["--named"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("missing name prefix"), "expected missing prefix error: {stderr:?}");
}

#[test]
fn named_duplicate_name_errors() {
    let (_, stderr, code) = run_stdin("a: echo 1\na: echo 2\n", &["--named"]);
    assert_ne!(code, 0);
    assert!(stderr.contains("duplicate"), "expected duplicate error: {stderr:?}");
}

#[test]
fn named_name_placeholder_uses_name() {
    let dir = TempDir::new("waffle_named_cmd");
    let pattern = dir.path().join("{name}.log");
    let (out, _, code) = run_stdin("mytask: echo hi\n", &["--named", "-o", pattern.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("mytask.log"), "expected name in filename: {out:?}");
    let content = std::fs::read_to_string(dir.path().join("mytask.log")).expect("log should exist");
    assert!(content.contains("mytask | hi"), "expected name label in log: {content:?}");
}

#[test]
fn without_named_flag_colon_lines_are_literal() {
    // No --named: the colon line is run verbatim as a command.
    let (out, _, code) = run_stdin("echo build: ok\n", &[]);
    assert_eq!(code, 0);
    assert!(out.contains("build: ok"), "expected literal command output: {out:?}");
}

#[test]
fn output_cmd_collapses_consecutive_underscores() {
    let dir = TempDir::new("waffle_output_collapse");
    let pattern = dir.path().join("{name}.log");
    let (out, _, code) = run_stdin("echo  hello   world\n", &["-o", pattern.to_str().unwrap()]);
    assert_eq!(code, 0);
    let log_name = out.lines().find(|l| l.ends_with(".log")).expect("expected log path line");
    let filename = std::path::Path::new(log_name.trim()).file_name().unwrap().to_str().unwrap();
    assert_eq!(filename, "echo_hello_world.log");
}
