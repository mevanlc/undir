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

#[test]
fn help_lists_cli_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_undir"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "Usage: undir [OPTIONS] <SRCDIR> [DSTDIR]\n       undir --completion <SHELL>"
        )
    );
    assert!(stdout.contains("--on-error <ON_ERROR>"));
    assert!(stdout.contains("-n, --dry-run"));
    assert!(stdout.contains("--completion <SHELL>"));
    assert!(stdout.contains("[possible values: bash, elvish, fish, pwsh, zsh]"));
}

#[test]
fn source_is_required_without_completion() {
    let output = Command::new(env!("CARGO_BIN_EXE_undir")).output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("<SRCDIR>"));
}

#[test]
fn completion_generates_scripts_for_supported_shells() {
    for shell in ["bash", "elvish", "fish", "pwsh", "zsh"] {
        let output = Command::new(env!("CARGO_BIN_EXE_undir"))
            .args(["--completion", shell])
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("undir"));
        assert!(stdout.contains("on-error"));
        assert!(stdout.contains("dry-run"));
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn on_error_controls_mutation_failures() {
    for policy in ["stop", "continue"] {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("a-collision"), "source").unwrap();
        fs::write(destination.join("a-collision"), "destination").unwrap();
        fs::write(source.join("b-movable"), "movable").unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_undir"))
            .args(["--preflight", "off", "--on-error", policy])
            .arg(&source)
            .arg(&destination)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("use --overwrite"));
        assert!(output.stdout.is_empty());
        assert_eq!(source.join("b-movable").exists(), policy == "stop");
        assert_eq!(destination.join("b-movable").exists(), policy == "continue");
        assert_eq!(
            fs::read_to_string(source.join("a-collision")).unwrap(),
            "source"
        );
        assert_eq!(
            fs::read_to_string(destination.join("a-collision")).unwrap(),
            "destination"
        );
    }
}

#[test]
fn dry_run_flags_preview_moves_without_changing_files() {
    for flag in ["-n", "--dry-run"] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let source = root.join("source");
        fs::create_dir_all(source.join("whole directory")).unwrap();
        fs::write(source.join(".hidden"), "hidden").unwrap();
        fs::write(source.join("whole directory/child"), "child").unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_undir"))
            .current_dir(&root)
            .args([flag, "source"])
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "move {:?} -> {:?}\nmove {:?} -> {:?}\nrmdir {:?}\n",
                source.join(".hidden"),
                root.join(".hidden"),
                source.join("whole directory"),
                root.join("whole directory"),
                source,
            )
        );
        assert_eq!(
            fs::read_to_string(source.join(".hidden")).unwrap(),
            "hidden"
        );
        assert_eq!(
            fs::read_to_string(source.join("whole directory/child")).unwrap(),
            "child"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }
}

#[test]
fn dry_run_previews_recursive_merge_overwrite_and_keep() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir_all(source.join("shared/nested")).unwrap();
    fs::create_dir_all(destination.join("shared/nested")).unwrap();
    fs::write(source.join("shared/nested/replaced"), "source").unwrap();
    fs::write(destination.join("shared/nested/replaced"), "destination").unwrap();
    fs::write(source.join("shared/new"), "new").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_undir"))
        .args(["-n", "--merge", "--overwrite", "--keep"])
        .arg(&source)
        .arg(&destination)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "overwrite {:?} -> {:?}\nrmdir {:?}\nmove {:?} -> {:?}\nrmdir {:?}\n",
            source.join("shared/nested/replaced"),
            destination.join("shared/nested/replaced"),
            source.join("shared/nested"),
            source.join("shared/new"),
            destination.join("shared/new"),
            source.join("shared"),
        )
    );
    assert_eq!(
        fs::read_to_string(source.join("shared/nested/replaced")).unwrap(),
        "source"
    );
    assert_eq!(
        fs::read_to_string(destination.join("shared/nested/replaced")).unwrap(),
        "destination"
    );
    assert_eq!(
        fs::read_to_string(source.join("shared/new")).unwrap(),
        "new"
    );
    assert!(!destination.join("shared/new").exists());
}

#[test]
fn dry_run_previews_destination_creation_without_creating_it() {
    for (flag, relative, operation) in [
        ("--mkdir", "destination", "mkdir"),
        ("--mkdirs", "parents/destination", "mkdirs"),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let source = root.join("source");
        let destination = root.join(relative);
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), "contents").unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_undir"))
            .args(["--dry-run", flag, "--preflight", "off"])
            .arg(&source)
            .arg(&destination)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "{operation} {destination:?}\nmove {:?} -> {:?}\nrmdir {source:?}\n",
                source.join("file"),
                destination.join("file"),
            )
        );
        assert_eq!(fs::read_to_string(source.join("file")).unwrap(), "contents");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }
}

#[test]
fn dry_run_reports_collisions_before_printing_a_plan_in_every_preflight_mode() {
    for preflight in ["fast", "full", "off"] {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        fs::create_dir_all(source.join("z-directory")).unwrap();
        fs::create_dir_all(destination.join("z-directory")).unwrap();
        fs::write(source.join("a-movable"), "movable").unwrap();
        fs::write(source.join("b-collision"), "source").unwrap();
        fs::write(destination.join("b-collision"), "destination").unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_undir"))
            .args(["-n", "--preflight", preflight, "--on-error", "continue"])
            .arg(&source)
            .arg(&destination)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("preflight:"));
        assert!(stderr.contains("use --overwrite"));
        assert_eq!(stderr.contains("use --merge"), preflight != "fast");
        assert_eq!(
            fs::read_to_string(source.join("a-movable")).unwrap(),
            "movable"
        );
        assert_eq!(
            fs::read_to_string(source.join("b-collision")).unwrap(),
            "source"
        );
        assert_eq!(
            fs::read_to_string(destination.join("b-collision")).unwrap(),
            "destination"
        );
        assert!(!destination.join("a-movable").exists());
        assert!(source.join("z-directory").is_dir());
    }
}
