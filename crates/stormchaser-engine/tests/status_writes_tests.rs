//! A run's or a step's status changes only in the state machines.
//!
//! Status changes made with plain SQL elsewhere bypassed the machines' checks: a step
//! put back to pending after a runner had taken it, a run deleted while it ran. The
//! machines (`step_machine/`, `workflow_machine.rs`) and the statements they save with
//! (`persistence.rs`) are the only places allowed to update either status. Creating a
//! row with its first status is not a change and is allowed anywhere.

use regex::Regex;
use std::path::{Path, PathBuf};

/// Where status updates may be written, relative to the repository root.
const ALLOWED: &[&str] = &[
    "crates/stormchaser-engine/src/step_machine/",
    "crates/stormchaser-engine/src/workflow_machine.rs",
    "crates/stormchaser-engine/src/persistence.rs",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// The places in `text` that update a run's or a step's status: an `UPDATE` of either
/// table whose `SET` clause names `status`. Returns their line numbers.
fn status_updates(text: &str) -> Vec<usize> {
    let update =
        Regex::new(r"(?is)UPDATE\s+(?:step_instances|workflow_runs)\b(.*?)\bWHERE\b").unwrap();
    let sets_status = Regex::new(r"(?i)(?:\bSET\s+|,\s*)status\s*=").unwrap();
    update
        .captures_iter(text)
        .filter(|c| sets_status.is_match(&c[1]))
        .map(|c| text[..c.get(0).unwrap().start()].matches('\n').count() + 1)
        .collect()
}

#[test]
fn only_the_state_machines_update_a_status() {
    let root = repo_root();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).unwrap() {
        rust_files(&entry.unwrap().path().join("src"), &mut files);
    }
    let mut outside = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if ALLOWED.iter().any(|allowed| relative.starts_with(allowed)) {
            continue;
        }
        for line in status_updates(&std::fs::read_to_string(&file).unwrap()) {
            outside.push(format!("{relative}:{line}"));
        }
    }
    assert!(
        outside.is_empty(),
        "status updates outside the state machines; add a transition instead:\n{}",
        outside.join("\n")
    );
}

#[test]
fn status_updates_are_recognised() {
    assert_eq!(
        status_updates("x\nsqlx::query(\"UPDATE step_instances SET status = $1 WHERE id = $2\")"),
        [2]
    );
    assert_eq!(
        status_updates("UPDATE workflow_runs\n SET error = $1, status = 'failed'\n WHERE id = $2"),
        [1]
    );
    // Not a status change
    assert!(
        status_updates("UPDATE step_instances SET dispatched_at = NOW() WHERE id = $1").is_empty()
    );
    assert!(status_updates("UPDATE runners SET status = $1 WHERE id = $2").is_empty());
    assert!(status_updates("SELECT status FROM step_instances WHERE id = $1").is_empty());
}
