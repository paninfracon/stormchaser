//! jq programs run in the engine's worker mode: a child process that is stopped when a
//! program runs too long or grows too large.

use serde_json::json;
use std::time::{Duration, Instant};
use stormchaser_engine::jq_sandbox;

fn use_engine_as_worker() {
    jq_sandbox::install_worker(env!("CARGO_BIN_EXE_stormchaser-engine").into());
    std::env::set_var(jq_sandbox::TIMEOUT_ENV, "3");
    std::env::set_var(jq_sandbox::MAX_MEMORY_ENV, "128");
}

#[tokio::test]
async fn an_ordinary_program_runs_in_the_worker() {
    use_engine_as_worker();
    let out = jq_sandbox::evaluate(
        ".items | map(.n * 2)",
        json!({"items": [{"n": 1}, {"n": 2}]}),
    )
    .await
    .unwrap();
    assert_eq!(out, vec![json!([2, 4])]);

    // Several outputs, a program that does not compile, and one that fails while running
    assert_eq!(
        jq_sandbox::evaluate(".[]", json!([1, "two"]))
            .await
            .unwrap(),
        vec![json!(1), json!("two")]
    );
    let err = jq_sandbox::evaluate("not jq (", json!(null))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("parse"), "{err}");
    assert!(jq_sandbox::evaluate("error(\"boom\")", json!(null))
        .await
        .is_err());
}

#[tokio::test]
async fn a_program_that_never_ends_is_stopped() {
    use_engine_as_worker();
    for endless in [
        // Produces nothing, for ever
        "last(repeat(1))",
        // Produces output for ever
        "repeat(1)",
    ] {
        let started = Instant::now();
        let err = jq_sandbox::evaluate(endless, json!(null))
            .await
            .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "`{endless}` was not stopped in time"
        );
        let text = err.to_string();
        assert!(
            text.contains("did not finish")
                || text.contains("more than")
                || text.contains("stopped"),
            "`{endless}`: {text}"
        );
    }
}

#[tokio::test]
async fn a_program_that_grows_without_end_is_stopped() {
    use_engine_as_worker();
    let started = Instant::now();
    let err = jq_sandbox::evaluate("[range(1000000000)] | length", json!(null))
        .await
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(20));
    let text = err.to_string();
    assert!(
        text.contains("stopped") || text.contains("did not finish"),
        "{text}"
    );
}

#[tokio::test]
async fn the_engine_keeps_working_while_a_program_spins() {
    use_engine_as_worker();
    // On a single-threaded runtime an in-process loop would freeze everything
    let spinning = tokio::spawn(jq_sandbox::evaluate("last(repeat(1))", json!(null)));
    let quick = tokio::time::timeout(
        Duration::from_secs(2),
        jq_sandbox::evaluate(".a", json!({"a": 7})),
    )
    .await
    .expect("an ordinary program was held up by a spinning one")
    .unwrap();
    assert_eq!(quick, vec![json!(7)]);
    assert!(spinning.await.unwrap().is_err());
}
