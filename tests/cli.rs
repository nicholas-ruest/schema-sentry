use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn write_pair(old: &str, new: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let directory = tempdir().expect("create temp directory");
    let old_path = directory.path().join("old.json");
    let new_path = directory.path().join("new.json");
    fs::write(&old_path, old).expect("write old schema");
    fs::write(&new_path, new).expect("write new schema");
    (directory, old_path, new_path)
}

#[test]
fn compatible_change_exits_zero() {
    let (_directory, old, new) = write_pair(
        r#"{"type":"string","maxLength":20}"#,
        r#"{"type":"string","maxLength":50}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_schema-sentry"))
        .args([old.as_os_str(), new.as_os_str()])
        .output()
        .expect("run binary");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "compatible\n");
}

#[test]
fn breaking_change_exits_two_with_actionable_text() {
    let (_directory, old, new) = write_pair(
        r#"{"type":"object","properties":{"id":{"type":"string"}}}"#,
        r#"{"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_schema-sentry"))
        .args([old.as_os_str(), new.as_os_str()])
        .output()
        .expect("run binary");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("incompatible"));
    assert!(stdout.contains("$.id"));
    assert!(stdout.contains("property_became_required"));
}

#[test]
fn json_output_is_machine_readable() {
    let (_directory, old, new) =
        write_pair(r#"{"type":["string","null"]}"#, r#"{"type":"string"}"#);
    let output = Command::new(env!("CARGO_BIN_EXE_schema-sentry"))
        .args([
            old.as_os_str(),
            new.as_os_str(),
            "--format".as_ref(),
            "json".as_ref(),
            "--allow-breaking".as_ref(),
        ])
        .output()
        .expect("run binary");
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse JSON report");
    assert_eq!(report["compatible"], false);
    assert_eq!(report["breaking_changes"], 1);
    assert_eq!(report["findings"][0]["rule"], "type_narrowed");
}

#[test]
fn malformed_input_exits_one() {
    let (_directory, old, new) = write_pair("not json", r#"{"type":"string"}"#);
    let output = Command::new(env!("CARGO_BIN_EXE_schema-sentry"))
        .args([old.as_os_str(), new.as_os_str()])
        .output()
        .expect("run binary");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid JSON"));
}
