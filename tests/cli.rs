//! End-to-end tests: run the real binary.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_logsleuth"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn groups_the_example_log() {
    let out = run(&["examples/app.log"], None);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(
        text.starts_with("12 lines · 4 INFO · 3 WARN · 4 ERROR · 1 FATAL"),
        "{text}"
    );
    assert!(
        text.contains(" 1. [FATAL] x1  connection pool exhausted (max <n>)"),
        "{text}"
    );
    assert!(
        text.contains(" 2. [ERROR] x3  payment provider timeout for order <uuid> after <n>"),
        "{text}"
    );
}

#[test]
fn json_output_is_valid() {
    let out = run(
        &["examples/app.log", "--format", "json", "--level", "error"],
        None,
    );
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    assert_eq!(json["total_lines"], 12);
    assert_eq!(json["groups"].as_array().unwrap().len(), 3);
    assert_eq!(json["groups"][1]["count"], 3);
}

#[test]
fn reads_from_stdin() {
    let out = run(&[], Some("ERROR bad byte \u{FFFD} here\nINFO ok\n"));
    assert!(out.status.success());
    assert!(stdout(&out).contains("[ERROR] x1"));
}

#[test]
fn fail_over_sets_exit_code() {
    assert_eq!(
        run(&["examples/app.log", "--fail-over", "10"], None)
            .status
            .code(),
        Some(0)
    );
    let out = run(&["examples/app.log", "--fail-over", "2"], None);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("5 error lines exceed the limit of 2"));
}

#[test]
fn missing_file_is_a_clear_error() {
    let out = run(&["does-not-exist.log"], None);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("does-not-exist.log"));
}
