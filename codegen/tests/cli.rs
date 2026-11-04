//! The generator as continuous integration and maintainers run it, on a copy of the workspace.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// A temporary workspace holding the vendored spec and, unless `generated` is `None`, a generated client.
fn copy(generated: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("spec")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::copy(
        workspace().join("spec/enablebanking-api.yaml"),
        dir.path().join("spec/enablebanking-api.yaml"),
    )
    .unwrap();
    if let Some(generated) = generated {
        std::fs::write(dir.path().join("src/generated.rs"), generated).unwrap();
    }
    dir
}

fn codegen(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codegen"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn checked_in() -> String {
    std::fs::read_to_string(workspace().join("src/generated.rs")).unwrap()
}

#[test]
fn generation_reproduces_the_checked_in_client() {
    let dir = copy(None);
    let output = codegen(dir.path(), &[]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/generated.rs")).unwrap(),
        checked_in()
    );
}

#[test]
fn check_accepts_a_fresh_client_and_rejects_a_stale_one() {
    let fresh = copy(Some(&checked_in()));
    let output = codegen(fresh.path(), &["--check"]);
    assert!(output.status.success(), "{output:?}");

    let stale = copy(Some(
        &checked_in().replace("start_authorization", "start_authorisation"),
    ));
    let output = codegen(stale.path(), &["--check"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("stale"),
        "{output:?}"
    );
    assert_eq!(
        std::fs::read_to_string(stale.path().join("src/generated.rs")).unwrap(),
        checked_in().replace("start_authorization", "start_authorisation"),
        "--check must not rewrite the client"
    );
}

#[test]
fn unknown_arguments_are_refused() {
    let dir = copy(Some(&checked_in()));
    let output = codegen(dir.path(), &["--chek"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("usage"),
        "{output:?}"
    );
}
