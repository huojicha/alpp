//! 已复现问题的 CLI 回归；源码与期望输出均来自 fixture。
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_alpp");

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn put(root: &Path, path: &str, name: &str) -> PathBuf {
    let dest = root.join(path);
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    fs::write(&dest, fixture(name)).unwrap();
    dest
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}

fn success(o: &Output) {
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn mirror_absolute_and_parent_inputs_never_modify_sources() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "src/input.js", "ts/regressions/input.js");
    let cwd = t.path().join("work");
    fs::create_dir(&cwd).unwrap();
    for input in [src.to_str().unwrap(), "../src/input.js", "../src"] {
        success(&run(&cwd, &["--no-config", "-w", "-o", "out", input]));
        assert_eq!(fs::read(&src).unwrap(), fixture("ts/regressions/input.js"));
        assert_eq!(
            fs::read(cwd.join("out/input.js")).unwrap(),
            fixture("ts/regressions/write.js")
        );
    }
}

#[test]
fn mirror_absolute_input_within_cwd_preserves_relative_layout() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "sub/input.js", "ts/regressions/input.js");
    success(&run(
        t.path(),
        &["--no-config", "-w", "-o", "out", src.to_str().unwrap()],
    ));
    assert_eq!(
        fs::read(t.path().join("out/sub/input.js")).unwrap(),
        fixture("ts/regressions/write.js")
    );
    assert_eq!(fs::read(src).unwrap(), fixture("ts/regressions/input.js"));
}

#[test]
fn mirror_repeated_runs_exclude_output_for_processed_and_passthrough_files() {
    let t = tempfile::tempdir().unwrap();
    put(t.path(), "input.js", "ts/regressions/input.js");
    put(t.path(), "notes.txt", "ts/regressions/input.js");
    put(t.path(), "binary.js", "ts/regressions/binary.js");
    for _ in 0..3 {
        success(&run(t.path(), &["--no-config", "-w", "-o", "out", "."]));
        assert!(!t.path().join("out/out").exists());
        assert_eq!(
            fs::read(t.path().join("out/input.js")).unwrap(),
            fixture("ts/regressions/write.js")
        );
        assert_eq!(
            fs::read(t.path().join("out/notes.txt")).unwrap(),
            fixture("ts/regressions/input.js")
        );
        assert_eq!(
            fs::read(t.path().join("out/binary.js")).unwrap(),
            fixture("ts/regressions/binary.js")
        );
    }
}

#[test]
fn mirror_rejects_source_root_before_writing() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.js", "ts/regressions/input.js");
    for input in [".", "input.js"] {
        let o = run(t.path(), &["--no-config", "-w", "-o", ".", input]);
        assert_eq!(o.status.code(), Some(1));
        assert_eq!(fs::read(&src).unwrap(), fixture("ts/regressions/input.js"));
    }
}

#[test]
fn mirror_rejects_conflicting_external_inputs_before_writing() {
    let t = tempfile::tempdir().unwrap();
    put(t.path(), "one/input.js", "ts/regressions/input.js");
    put(t.path(), "two/input.js", "ts/regressions/write.js");
    let cwd = t.path().join("work");
    fs::create_dir(&cwd).unwrap();
    let o = run(
        &cwd,
        &["--no-config", "-w", "-o", "out", "../one", "../two"],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(!cwd.join("out").exists());
}

#[test]
fn mirror_explicit_unknown_file_is_not_overwritten_by_passthrough() {
    let t = tempfile::tempdir().unwrap();
    put(t.path(), "input.txt", "ts/regressions/input.js");
    success(&run(
        t.path(),
        &["--no-config", "-w", "-o", "out", "input.txt", "."],
    ));
    assert_eq!(
        fs::read(t.path().join("out/input.txt")).unwrap(),
        fixture("ts/regressions/write.js")
    );
}

#[cfg(unix)]
#[test]
fn mirror_rejects_destination_symlink_escape_before_writing() {
    let t = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    put(t.path(), "sub/input.js", "ts/regressions/input.js");
    let protected = put(external.path(), "input.js", "ts/regressions/input.js");
    fs::create_dir(t.path().join("out")).unwrap();
    std::os::unix::fs::symlink(external.path(), t.path().join("out/sub")).unwrap();
    let o = run(t.path(), &["--no-config", "-w", "-o", "out", "sub"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        fs::read(protected).unwrap(),
        fixture("ts/regressions/input.js")
    );
}

#[test]
fn config_discovery_from_subdirectory_matches_absolute_anchor() {
    let t = tempfile::tempdir().unwrap();
    put(t.path(), ".alpp", "ts/regressions/deny.alpp");
    let src = put(t.path(), "src/input.js", "ts/regressions/input.js");
    let cwd = src.parent().unwrap();
    for input in [
        ".",
        "input.js",
        cwd.to_str().unwrap(),
        src.to_str().unwrap(),
    ] {
        let o = run(cwd, &["--check", input]);
        assert_eq!(o.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&o.stderr).contains("E8"));
        assert!(o.stdout.is_empty());
    }
}

#[test]
fn config_defaults_are_loaded_from_parent_of_cwd() {
    let t = tempfile::tempdir().unwrap();
    put(t.path(), ".alpp", "ts/regressions/defaults.alpp");
    let src = put(t.path(), "src/input.js", "ts/regressions/input.js");
    success(&run(src.parent().unwrap(), &["-w", "input.js"]));
    assert_eq!(
        fs::read(src).unwrap(),
        fixture("ts/regressions/write-flag.js")
    );
}

#[test]
fn write_does_not_clobber_existing_temp_filename() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.js", "ts/regressions/input.js");
    let other = put(t.path(), "input.alpp-tmp", "ts/regressions/keywords.js");
    success(&run(t.path(), &["--no-config", "-w", "input.js"]));
    assert_eq!(fs::read(src).unwrap(), fixture("ts/regressions/write.js"));
    assert_eq!(
        fs::read(other).unwrap(),
        fixture("ts/regressions/keywords.js")
    );
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn write_and_mirror_preserve_executable_mode() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.sh", "shell/regressions/input.sh");
    fs::set_permissions(&src, fs::Permissions::from_mode(0o755)).unwrap();
    success(&run(
        t.path(),
        &["--no-config", "-w", "-o", "out", "input.sh"],
    ));
    success(&run(t.path(), &["--no-config", "-w", "input.sh"]));
    for path in [src, t.path().join("out/input.sh")] {
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::read(path).unwrap(),
            fixture("shell/regressions/write.sh")
        );
    }
}

#[test]
fn scanner_requires_exact_keyword_boundary() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "keywords.js", "ts/regressions/keywords.js");
    let strict = run(
        t.path(),
        &["--no-config", "--strict", "--check", "keywords.js"],
    );
    assert_eq!(strict.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&strict.stderr).contains("E4"));
    success(&run(t.path(), &["--no-config", "-w", "keywords.js"]));
    assert_eq!(
        fs::read(src).unwrap(),
        fixture("ts/regressions/write-keywords.js")
    );
}

#[test]
fn empty_prefix_fails_without_rewriting() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.js", "ts/regressions/input.js");
    let o = run(t.path(), &["--no-config", "--prefix", "", "-w", "input.js"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(fs::read(src).unwrap(), fixture("ts/regressions/input.js"));
}

#[test]
fn explicit_and_discovered_files_share_binary_and_size_limits() {
    let t = tempfile::tempdir().unwrap();
    let binary = put(t.path(), "binary.js", "ts/regressions/binary.js");
    let src = put(t.path(), "input.js", "ts/regressions/input.js");
    for input in [".", "binary.js"] {
        success(&run(t.path(), &["--no-config", "-w", input]));
        assert_eq!(
            fs::read(&binary).unwrap(),
            fixture("ts/regressions/binary.js")
        );
    }
    // 重新放回源码，验证大小过滤在两种入口一致。
    for input in [".", "input.js"] {
        put(t.path(), "input.js", "ts/regressions/input.js");
        success(&run(
            t.path(),
            &["--no-config", "--max-size", "1", "-w", input],
        ));
        assert_eq!(fs::read(&src).unwrap(), fixture("ts/regressions/input.js"));
    }
}

#[test]
fn custom_prefix_processes_unknown_extensions_in_directory_walk() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.txt", "ts/regressions/custom.txt");
    success(&run(
        t.path(),
        &["--no-config", "--prefix", ";#", "-w", "."],
    ));
    assert_eq!(
        fs::read(src).unwrap(),
        fixture("ts/regressions/write-custom.txt")
    );
}

#[test]
fn diff_for_retained_unterminated_tail_applies_to_write_output() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.js", "ts/regressions/input.js");
    let o = run(t.path(), &["--no-config", "input.js"]);
    success(&o);
    assert_eq!(o.stdout, fixture("ts/regressions/input.diff"));
    let patch = t.path().join("change.patch");
    fs::write(&patch, &o.stdout).unwrap();
    let applied = Command::new("git")
        .args(["apply", "--no-index"])
        .arg(&patch)
        .current_dir(t.path())
        .output()
        .unwrap();
    success(&applied);
    assert_eq!(fs::read(src).unwrap(), fixture("ts/regressions/write.js"));
}

#[test]
fn stdout_write_failure_is_exit_1() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.js", "ts/regressions/input.js");
    let o = Command::new(BIN)
        .args(["--no-config", "input.js"])
        .current_dir(t.path())
        .stdout(Stdio::from(fs::File::open(&src).unwrap()))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("stdout"));
    assert_eq!(fs::read(src).unwrap(), fixture("ts/regressions/input.js"));
}

#[test]
fn check_rejects_write_and_output_without_side_effects() {
    let t = tempfile::tempdir().unwrap();
    let src = put(t.path(), "input.js", "ts/regressions/input.js");
    let o = run(
        t.path(),
        &["--no-config", "--check", "-w", "-o", "out", "input.js"],
    );
    assert!(!o.status.success());
    assert!(!t.path().join("out").exists());
    assert_eq!(fs::read(src).unwrap(), fixture("ts/regressions/input.js"));
}

#[test]
fn batches_preserve_diff_and_error_order_across_thread_counts() {
    let t = tempfile::tempdir().unwrap();
    for i in 0..65 {
        put(t.path(), &format!("f{i:02}.js"), "ts/regressions/input.js");
    }
    for mode in [None, Some("--check"), Some("--gate")] {
        let mut args = vec!["--no-config", "-j", "1", "."];
        if let Some(mode) = mode {
            args.push(mode);
        }
        let one = run(t.path(), &args);
        args[2] = "4";
        let four = run(t.path(), &args);
        assert_eq!(one.status.code(), four.status.code());
        assert_eq!(one.stdout, four.stdout);
        assert_eq!(one.stderr, four.stderr);
    }
}
