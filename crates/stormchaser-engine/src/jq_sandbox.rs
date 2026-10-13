//! Runs workflow-supplied jq programs where they can be stopped.
//!
//! A jq program comes from a workflow author and can loop for ever or build a value
//! without end. The jq interpreter cannot be interrupted from outside, and a thread
//! cannot be killed, so run in the engine's own process such a program would hold a
//! thread and grow the engine's memory until the engine died, taking every other run
//! with it.
//!
//! Programs are therefore run in a child process: this same executable, started in a
//! worker mode that does nothing but evaluate one program. The child gets none of the
//! parent's environment, has a ceiling on its memory, and is killed if it does not answer
//! in time.
//!
//! A service enables this by calling [`run_worker_if_requested`] first thing in `main`
//! and [`install_current_exe_as_worker`] during start-up. Without a worker installed
//! (unit tests, and nothing else) programs run in the calling process, unbounded.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Seconds a jq program may run. Defaults to 10.
pub const TIMEOUT_ENV: &str = "STORMCHASER_JQ_TIMEOUT_SECS";

/// Memory a jq program may use, in MiB. Defaults to 512.
pub const MAX_MEMORY_ENV: &str = "STORMCHASER_JQ_MAX_MEMORY_MB";

/// Largest result a jq program may return.
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024;

/// Set in the child's environment to put the executable in worker mode.
const WORKER_ENV: &str = "STORMCHASER_JQ_WORKER";

static WORKER: OnceLock<PathBuf> = OnceLock::new();

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// Uses `exe` as the worker for every later [`evaluate`]. The first call wins.
pub fn install_worker(exe: PathBuf) {
    let _ = WORKER.set(exe);
}

/// Uses the running executable as the worker. Its `main` must call
/// [`run_worker_if_requested`] before doing anything else.
pub fn install_current_exe_as_worker() -> Result<()> {
    install_worker(std::env::current_exe().context("Cannot locate the running executable")?);
    Ok(())
}

/// Evaluates `program` against `input` and returns every value it outputs.
pub async fn evaluate(program: &str, input: Value) -> Result<Vec<Value>> {
    match WORKER.get() {
        Some(exe) => evaluate_in_worker(exe, program, input).await,
        None => {
            let program = program.to_string();
            tokio::task::spawn_blocking(move || evaluate_in_process(&program, input))
                .await
                .map_err(|e| anyhow!("jq task failed: {}", e))?
        }
    }
}

async fn evaluate_in_worker(exe: &PathBuf, program: &str, input: Value) -> Result<Vec<Value>> {
    let timeout = Duration::from_secs(env_u64(TIMEOUT_ENV, 10));
    let request = serde_json::to_vec(&serde_json::json!({ "program": program, "input": input }))?;

    let mut child = tokio::process::Command::new(exe)
        // Nothing of the engine's environment: it holds the engine's own credentials
        .env_clear()
        .env(WORKER_ENV, "1")
        .env(MAX_MEMORY_ENV, env_u64(MAX_MEMORY_ENV, 512).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Dropping the child, which a timeout does, kills it
        .kill_on_drop(true)
        .spawn()
        .context("Failed to start the jq worker")?;

    let mut stdin = child.stdin.take().context("jq worker has no stdin")?;
    let mut stdout = child.stdout.take().context("jq worker has no stdout")?;

    let exchange = async {
        // Written from its own task: a worker that fills its output pipe before it has
        // read all of its input must not deadlock against us
        let write = tokio::spawn(async move {
            let _ = stdin.write_all(&request).await;
            let _ = stdin.shutdown().await;
        });
        let mut output = Vec::new();
        (&mut stdout)
            .take(MAX_OUTPUT_BYTES + 1)
            .read_to_end(&mut output)
            .await?;
        let _ = write.await;
        if output.len() as u64 > MAX_OUTPUT_BYTES {
            bail!(
                "jq program produced more than {} bytes of output",
                MAX_OUTPUT_BYTES
            );
        }
        let status = child.wait().await?;
        Ok((status, output))
    };

    let (status, output) = match tokio::time::timeout(timeout, exchange).await {
        Ok(result) => result?,
        Err(_) => bail!(
            "jq program did not finish within {} seconds",
            timeout.as_secs()
        ),
    };
    if !status.success() {
        bail!("jq program was stopped: it ran out of memory or crashed");
    }

    let answer: Value =
        serde_json::from_slice(&output).context("jq worker returned an unreadable answer")?;
    if let Some(error) = answer.get("error").and_then(Value::as_str) {
        bail!("{}", error);
    }
    match answer.get("ok") {
        Some(Value::Array(values)) => Ok(values.clone()),
        _ => bail!("jq worker returned an unreadable answer"),
    }
}

/// Evaluates a jq program in the calling process, with no limit on time or memory.
pub fn evaluate_in_process(program: &str, input: Value) -> Result<Vec<Value>> {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{data, Ctx, Vars};
    use jaq_json::Val;

    let loader = Loader::new(
        jaq_core::defs()
            .chain(jaq_std::defs())
            .chain(jaq_json::defs()),
    );
    let arena = Arena::default();
    let modules = loader
        .load(
            &arena,
            File {
                code: program,
                path: (),
            },
        )
        .map_err(|e| anyhow!("Failed to parse jq program: {:?}", e))?;

    let filter = jaq_core::Compiler::default()
        .with_funs(crate::query::jq_funs())
        .compile(modules)
        .map_err(|e| anyhow!("Failed to compile jq program: {:?}", e))?;

    let input = crate::query::json_to_jq_val(input)?;
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    filter
        .id
        .run((ctx, input))
        .map(crate::query::jq_output_to_json)
        .collect()
}

/// If this process was started as a jq worker, evaluates the one program on its standard
/// input, writes the answer to standard output and exits. Otherwise returns at once.
///
/// Call it before anything else in `main`, in particular before an async runtime is
/// started: the worker needs none of it, and the memory ceiling should cover as little
/// that is not the program as possible.
pub fn run_worker_if_requested() {
    if std::env::var_os(WORKER_ENV).is_none() {
        return;
    }
    limit_memory(env_u64(MAX_MEMORY_ENV, 512));

    let answer = worker_answer();
    let _ = std::io::Write::write_all(
        &mut std::io::stdout(),
        &serde_json::to_vec(&answer).unwrap_or_default(),
    );
    std::process::exit(0);
}

fn worker_answer() -> Value {
    let mut request = Vec::new();
    if let Err(e) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut request) {
        return serde_json::json!({ "error": format!("jq worker could not read its input: {e}") });
    }
    let request: Value = match serde_json::from_slice(&request) {
        Ok(request) => request,
        Err(e) => {
            return serde_json::json!({ "error": format!("jq worker got an unreadable request: {e}") })
        }
    };
    let program = request["program"].as_str().unwrap_or_default();
    match evaluate_in_process(program, request["input"].clone()) {
        Ok(values) => serde_json::json!({ "ok": values }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

/// Caps the memory this process may allocate. An allocation beyond it fails, which
/// ends the process.
#[cfg(unix)]
fn limit_memory(megabytes: u64) {
    let bytes = megabytes.saturating_mul(1024 * 1024) as libc::rlim_t;
    let limit = libc::rlimit {
        rlim_cur: bytes,
        rlim_max: bytes,
    };
    // SAFETY: `setrlimit` only reads the struct it is given, which lives for the call.
    // RLIMIT_DATA covers the heap and anonymous mappings, which is where a jq value
    // grows; it leaves the executable's own mappings out of the count.
    unsafe {
        libc::setrlimit(libc::RLIMIT_DATA, &limit);
    }
}

#[cfg(not(unix))]
fn limit_memory(_megabytes: u64) {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_program_returns_every_value_it_outputs() {
        assert_eq!(
            evaluate_in_process(
                ".items[] | .name",
                json!({"items": [{"name": "a"}, {"name": "b"}]})
            )
            .unwrap(),
            vec![json!("a"), json!("b")]
        );
        assert!(evaluate_in_process("this is not jq (", json!(null)).is_err());
        // The engine's environment is not readable from a program
        assert!(evaluate_in_process("env", json!(null)).is_err());
    }
}
