use std::fs;
use std::process::Command;

#[test]
fn successful_cli_is_silent() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("file"), "contents").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_undir"))
        .arg(&source)
        .arg(&destination)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn destination_defaults_to_current_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("moved"), "contents").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_undir"))
        .current_dir(temporary.path())
        .arg("source")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(temporary.path().join("moved")).unwrap(),
        "contents"
    );
}

#[test]
fn mkdir_and_mkdirs_conflict_in_clap() {
    let output = Command::new(env!("CARGO_BIN_EXE_undir"))
        .args(["source", "destination", "--mkdir", "--mkdirs"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}
