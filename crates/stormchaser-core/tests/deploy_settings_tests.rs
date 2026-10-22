//! Every environment variable the Helm charts and docker-compose set on a Stormchaser
//! service is one that service reads.
//!
//! A setting nobody reads fails silently: a misspelt name, or one a refactor renamed,
//! leaves the service on its default, which for a security setting is the unsafe
//! choice the operator meant to change. The check is by name: a service "reads" a
//! variable when its name appears as a string literal in the service's crate or in
//! one of the workspace crates it depends on.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Each service: its crate, its Helm template and its docker-compose service, if it
/// runs in compose.
const SERVICES: &[(&str, &str, Option<&str>)] = &[
    (
        "stormchaser-api",
        "deploy/charts/stormchaser-orchestration/templates/api-deployment.yaml",
        Some("orchestration-api"),
    ),
    (
        "stormchaser-engine",
        "deploy/charts/stormchaser-orchestration/templates/engine-deployment.yaml",
        Some("orchestration-engine"),
    ),
    (
        "stormchaser-query",
        "deploy/charts/stormchaser-orchestration/templates/query-deployment.yaml",
        Some("orchestration-query"),
    ),
    (
        "stormchaser-web",
        "deploy/charts/stormchaser-orchestration/templates/web-deployment.yaml",
        Some("stormchaser-web"),
    ),
    (
        "stormchaser-runner-k8s",
        "deploy/charts/stormchaser-runner-k8s/templates/deployment.yaml",
        // Its compose service only builds the image
        None,
    ),
    (
        "stormchaser-runner-docker",
        "deploy/charts/stormchaser-runner-docker/templates/daemonset.yaml",
        Some("docker-runner"),
    ),
];

/// Variables a library reads on the service's behalf, so their names are not in its
/// code: (service crate, variable, the library).
const READ_BY_LIBRARIES: &[(&str, &str, &str)] = &[
    (
        "stormchaser-query",
        "RUST_LOG",
        "tracing_subscriber::fmt::init",
    ),
    ("stormchaser-engine", "AWS_DEFAULT_REGION", "aws-config"),
    (
        "stormchaser-runner-docker",
        "DOCKER_HOST",
        "bollard's connect_with_local_defaults",
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn is_variable_name(s: &str) -> bool {
    s.len() >= 3
        && s.starts_with(|c: char| c.is_ascii_uppercase())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// The workspace crates `krate` needs at run time, itself included: the `stormchaser-*`
/// entries of its `[dependencies]`, followed transitively. Dev-dependencies don't count.
fn runtime_crates(root: &Path, krate: &str, found: &mut BTreeSet<String>) {
    if !found.insert(krate.to_string()) {
        return;
    }
    let manifest = read(&root.join("crates").join(krate).join("Cargo.toml"));
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
        } else if in_dependencies {
            let name = line.split(['=', ' ', '.']).next().unwrap_or("");
            if name.starts_with("stormchaser-") {
                runtime_crates(root, name, found);
            }
        }
    }
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

/// Every string literal that looks like a variable name, in the crates' sources.
fn names_in_code(root: &Path, crates: &BTreeSet<String>) -> BTreeSet<String> {
    let mut files = Vec::new();
    for krate in crates {
        rust_files(&root.join("crates").join(krate).join("src"), &mut files);
    }
    let mut names = BTreeSet::new();
    for file in files {
        let text = read(&file);
        // Splitting on quotes puts each literal's contents at an odd index
        for (i, piece) in text.split('"').enumerate() {
            if i % 2 == 1 && is_variable_name(piece) {
                names.insert(piece.to_string());
            }
        }
    }
    names
}

/// The variables a Helm template sets: `- name: X` entries with an upper-case name
/// (container and volume names are lower case).
fn names_in_template(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("- name:"))
        .map(|name| name.trim().trim_matches('"').to_string())
        .filter(|name| is_variable_name(name))
        .collect()
}

/// The variables docker-compose sets on one service, in either `environment:` form.
fn names_in_compose(text: &str, service: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut in_service = false;
    let mut in_environment = false;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if indent == 2 {
            in_service = trimmed == format!("{service}:");
            in_environment = false;
        } else if in_service && indent == 4 {
            in_environment = trimmed == "environment:";
        } else if in_service && in_environment && indent >= 6 {
            let entry = trimmed.trim_start_matches("- ");
            let name = entry.split([':', '=']).next().unwrap_or("").trim();
            if is_variable_name(name) {
                names.insert(name.to_string());
            }
        }
    }
    names
}

#[test]
fn every_setting_a_deployment_file_sets_is_read_by_its_service() {
    let root = repo_root();
    let compose = read(&root.join("docker-compose.yml"));
    let mut unread = Vec::new();
    for (krate, template, compose_service) in SERVICES {
        let mut crates = BTreeSet::new();
        runtime_crates(&root, krate, &mut crates);
        let mut known = names_in_code(&root, &crates);
        known.extend(
            READ_BY_LIBRARIES
                .iter()
                .filter(|(service, _, _)| service == krate)
                .map(|(_, name, _)| name.to_string()),
        );

        let in_template = names_in_template(&read(&root.join(template)));
        assert!(!in_template.is_empty(), "found no settings in {template}");
        for name in in_template.difference(&known) {
            unread.push(format!("{template}: {name} (on {krate})"));
        }
        if let Some(compose_service) = compose_service {
            let in_compose = names_in_compose(&compose, compose_service);
            assert!(
                !in_compose.is_empty(),
                "found no settings for the compose service {compose_service}"
            );
            for name in in_compose.difference(&known) {
                unread.push(format!(
                    "docker-compose.yml {compose_service}: {name} (on {krate})"
                ));
            }
        }
    }
    assert!(
        unread.is_empty(),
        "set, but read by nothing in the service:\n{}",
        unread.join("\n")
    );
}

#[test]
fn the_parsers_find_what_they_should() {
    let template = "env:\n  - name: NATS_URL\n    value: x\n  - name: \"RUST_LOG\"\ncontainers:\n  - name: api\n";
    assert_eq!(
        names_in_template(template),
        BTreeSet::from(["NATS_URL".to_string(), "RUST_LOG".to_string()])
    );
    let compose = "services:\n  api:\n    image: x\n    environment:\n      NATS_URL: nats://n\n      # COMMENTED: x\n    ports:\n      - \"80:80\"\n  web:\n    environment:\n      - RUST_LOG=info\n";
    assert_eq!(
        names_in_compose(compose, "api"),
        BTreeSet::from(["NATS_URL".to_string()])
    );
    assert_eq!(
        names_in_compose(compose, "web"),
        BTreeSet::from(["RUST_LOG".to_string()])
    );
}
