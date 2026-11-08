//! Runs the CLI on folders on disk and checks which rule files it reads.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Creates an empty folder under the system temporary folder for the test `name`.
fn scratch_folder(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("nomenclator-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("the temporary folder is writable");
    folder
}

/// Writes `contents` to `path`, and creates the folders above it.
fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("the path has a parent"))
        .expect("the folder is writable");
    std::fs::write(path, contents).expect("the file is writable");
}

/// Runs `nomenclator interpret` on `library` with `arguments` and returns the paths of
/// the excluded entries.
fn excluded_paths(library: &Path, arguments: &[&str]) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_nomenclator"))
        .arg("interpret")
        .arg(library)
        .args(["--library", "movies", "--format", "json"])
        .args(arguments)
        .output()
        .expect("the CLI runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the output is JSON");
    result["excluded_entries"]
        .as_array()
        .expect("the result lists excluded entries")
        .iter()
        .map(|entry| entry["path"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn interpret_reads_marker_added_by_overlay() {
    let root = scratch_folder("overlay-marker");
    let library = root.join("library");
    write(&library.join("Drafts/Arrival (2016).mkv"), "");
    write(&library.join("Drafts/.skip"), "");
    write(&library.join("Arrival (2016)/Arrival (2016).mkv"), "");
    let overlay = root.join("skip-marker.yaml");
    write(
        &overlay,
        "name: skip-marker\nno_media_markers:\n  add: [.skip]\n",
    );

    let without = excluded_paths(&library, &[]);
    let with = excluded_paths(
        &library,
        &["--overlay", overlay.to_str().unwrap_or_default()],
    );

    assert!(!without.contains(&"Drafts/".to_owned()), "{without:?}");
    assert!(with.contains(&"Drafts/".to_owned()), "{with:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn interpret_reads_marker_regardless_of_case() {
    let root = scratch_folder("marker-case");
    let library = root.join("library");
    write(&library.join("Drafts/Arrival (2016).mkv"), "");
    write(&library.join("Drafts/.NOMEDIA"), "");

    let excluded = excluded_paths(&library, &[]);

    assert!(excluded.contains(&"Drafts/".to_owned()), "{excluded:?}");
    let _ = std::fs::remove_dir_all(&root);
}
