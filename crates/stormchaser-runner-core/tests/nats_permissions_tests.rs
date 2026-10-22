//! The runner NATS user's publish allow-lists (the compose server's config and the
//! umbrella chart's) cover every subject a runner publishes. A subject they leave out is
//! refused by the server, and runners do not always notice.

use stormchaser_model::nats::NatsSubject;

/// Every subject a runner publishes, for a run on shard 7.
fn runner_subjects() -> Vec<String> {
    let shard = Some(7);
    let mut subjects: Vec<String> = [
        NatsSubject::RunnerHeartbeat,
        NatsSubject::RunnerOffline,
        NatsSubject::StepInitializing(shard),
        NatsSubject::StepRunning(shard),
        NatsSubject::StepCompleted(shard),
        NatsSubject::StepFailed(shard),
        NatsSubject::StepUnpackingSfs(shard),
        NatsSubject::StepPackingSfs(shard),
    ]
    .iter()
    .map(|s| s.as_str().into_owned())
    .collect();
    subjects.extend(
        [
            stormchaser_runner_core::runner::REGISTER_SUBJECT,
            stormchaser_model::nats::STEP_QUERY_SUBJECT,
            "stormchaser.rpc.step.artifact_auth",
            "stormchaser.rpc.runner.session",
        ]
        .map(String::from),
    );
    subjects
}

/// Whether `subject` matches the NATS subject `pattern` (`*` one token, `>` the rest).
fn matches(pattern: &str, subject: &str) -> bool {
    let mut subject_tokens = subject.split('.');
    for token in pattern.split('.') {
        match (token, subject_tokens.next()) {
            (">", Some(_)) => return true,
            ("*", Some(_)) => {}
            (literal, Some(actual)) if literal == actual => {}
            _ => return false,
        }
    }
    subject_tokens.next().is_none()
}

/// The quoted subjects in the runner user's publish allow-list of `config`, read from
/// the line naming the user to the first `deny`.
fn runner_allow_list(config: &str, user_line: &str) -> Vec<String> {
    let start = config
        .find(user_line)
        .expect("the runner user is configured");
    let rest = &config[start..];
    let block = &rest[..rest.find("deny").expect("a deny list follows")];
    block
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn check(file: &str, user_line: &str) {
    let path = format!("{}/../../{}", env!("CARGO_MANIFEST_DIR"), file);
    let config = std::fs::read_to_string(&path).unwrap();
    let allowed = runner_allow_list(&config, user_line);
    assert!(allowed.len() > 5, "{file}: {allowed:?}");
    for subject in runner_subjects() {
        assert!(
            allowed.iter().any(|pattern| matches(pattern, &subject)),
            "{file} does not let runners publish {subject}"
        );
    }
}

#[test]
fn the_compose_nats_server_lets_runners_publish_what_they_publish() {
    check("deploy/nats/nats-server.conf", "user: stormchaser-runner");
}

#[test]
fn the_umbrella_chart_lets_runners_publish_what_they_publish() {
    check(
        "deploy/charts/stormchaser/values.yaml",
        "- user: stormchaser-runner",
    );
}

#[test]
fn subject_patterns_match_as_nats_does() {
    assert!(matches(
        "stormchaser.v1.*.step.failed",
        "stormchaser.v1.7.step.failed"
    ));
    assert!(!matches(
        "stormchaser.v1.step.packing_sfs",
        "stormchaser.v1.7.step.packing_sfs"
    ));
    assert!(matches("$JS.ACK.stormchaser.>", "$JS.ACK.stormchaser.a.b"));
    assert!(!matches("stormchaser.v1.*", "stormchaser.v1.a.b"));
}
