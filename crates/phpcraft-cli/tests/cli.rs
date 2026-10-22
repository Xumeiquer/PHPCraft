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
fn match_expression_and_unhandled_case() {
    let output = phpcraft()
        .args([
            "-r",
            r#"echo match (2) { 1, 2 => "small", default => "other" };"#,
        ])
        .output()
        .expect("run phpcraft");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "small");

    let output = phpcraft()
        .args(["-r", r#"match (2) { 1 => "one" };"#])
        .output()
        .expect("run phpcraft");
    assert_eq!(output.status.code(), Some(255));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("UnhandledMatchError"));
}

#[test]
fn closures_run_and_by_reference_captures_are_rejected() {
    let output = phpcraft()
        .args([
            "-r",
            r#"$n = 2; $f = fn($x) => $x + $n; echo $f(3), "|", (function ($x) { return $x * 2; })(4);"#,
        ])
        .output()
        .expect("run phpcraft");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5|8");

    let output = phpcraft()
        .args([
            "-r",
            r#"$x = 1; $f = function () use (&$x) { return $x; };"#,
        ])
        .output()
        .expect("run phpcraft");
    assert_eq!(output.status.code(), Some(255));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("by-reference closure captures are not supported"));
}

#[test]
fn classes_and_exceptions_run() {
    let output = phpcraft()
        .args([
            "-r",
            r#"class A { public $x = 1; function __construct($x) { $this->x = $x; } function get() { return $this->x; } } $o = new A(4); try { throw new RuntimeException("boom"); } catch (Exception $e) { echo $o->get(), "|", $e->getMessage(); }"#,
        ])
        .output()
        .expect("run phpcraft");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "4|boom");

    let output = phpcraft()
        .args(["-r", r#"throw new Exception("uncaught");"#])
        .output()
        .expect("run phpcraft");
    assert_eq!(output.status.code(), Some(255));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Uncaught Exception: uncaught"));
    assert!(stderr.contains("Stack trace:"));
}

#[test]
fn extreme_integers_do_not_panic_the_interpreter() {
    for code in [
        "str_repeat('a', PHP_INT_MAX);",
        "str_pad('a', PHP_INT_MAX);",
        "array_fill(0, PHP_INT_MAX, 1);",
        "number_format(1.5, PHP_INT_MAX);",
        "sprintf('%999999999999999999s', 'x');",
    ] {
        let output = phpcraft()
            .args(["-r", code])
            .output()
            .expect("run phpcraft");
        assert_eq!(output.status.code(), Some(255), "{code}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("panicked at"), "{code}: {stderr}");
        assert!(output.stdout.is_empty(), "{code}");
    }
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
