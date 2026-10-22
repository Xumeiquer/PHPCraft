//! End-to-end CLI behaviour, exercised through the built binary.

use std::io::Write;
use std::process::{Command, Stdio};

fn phpcraft() -> Command {
    Command::new(env!("CARGO_BIN_EXE_phpcraft"))
}

#[test]
fn run_code_without_tags() {
    let output = phpcraft()
        .args(["-r", "echo 6 * 7, PHP_EOL;"])
        .output()
        .expect("run phpcraft");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
}

#[test]
fn run_code_exposes_script_arguments() {
    let output = phpcraft()
        .args([
            "-r",
            r#"echo $argc, ":", $argv[0], ":", $argv[1], ":", $argv[2], PHP_EOL;"#,
            "foo",
            "bar",
        ])
        .output()
        .expect("run phpcraft");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "3:Standard input code:foo:bar\n"
    );
}

#[test]
fn diagnostics_go_to_stderr_with_status_255() {
    let output = phpcraft()
        .args(["-r", "nope();"])
        .output()
        .expect("run phpcraft");
    assert_eq!(output.status.code(), Some(255));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("undefined function nope"));
}

#[test]
fn reads_script_from_stdin() {
    let mut child = phpcraft()
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn phpcraft");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"<?php echo strtoupper('stdin'), PHP_EOL;")
        .expect("write script");
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "STDIN\n");
}
