use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;
use wasmtime::*;

/// Largest WASM module the engine will download or decode.
const MAX_WASM_MODULE_BYTES: usize = 64 * 1024 * 1024;

/// Largest result a WASM function may hand back.
const MAX_WASM_RESULT_BYTES: usize = 10 * 1024 * 1024;

/// Default wall-clock limit for loading and running a module
/// (`STORMCHASER_WASM_TIMEOUT_SECS`).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default cap on a module's linear memory (`STORMCHASER_WASM_MAX_MEMORY_MB`).
const DEFAULT_MAX_MEMORY_MB: usize = 256;

/// How much fuel a guest burns between yields to the async executor. Yielding is what lets
/// the time limit interrupt a guest that never returns.
const FUEL_YIELD_INTERVAL: u64 = 100_000;

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Utility for executing WebAssembly (WASM) modules using Wasmtime.
///
/// Modules are untrusted: they get no host imports, a capped linear memory and a wall-clock
/// limit, and they run in slices so a busy guest cannot hold an executor thread.
pub struct WasmExecutor {
    engine: Engine,
    timeout: Duration,
    max_memory_bytes: usize,
}

impl Default for WasmExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmExecutor {
    /// Creates a new `WasmExecutor` with limits taken from the environment.
    pub fn new() -> Self {
        Self::with_limits(
            Duration::from_secs(env_or(
                "STORMCHASER_WASM_TIMEOUT_SECS",
                DEFAULT_TIMEOUT_SECS,
            )),
            env_or("STORMCHASER_WASM_MAX_MEMORY_MB", DEFAULT_MAX_MEMORY_MB)
                .saturating_mul(1024 * 1024),
        )
    }

    /// Creates a new `WasmExecutor` with explicit limits.
    pub fn with_limits(timeout: Duration, max_memory_bytes: usize) -> Self {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).expect("Failed to create Wasmtime engine");
        Self {
            engine,
            timeout,
            max_memory_bytes,
        }
    }

    /// Execute.
    pub async fn execute(
        &self,
        module_path: &str,
        function_name: &str,
        input: Value,
    ) -> Result<Value> {
        match tokio::time::timeout(
            self.timeout,
            self.execute_unbounded(module_path, function_name, input),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => anyhow::bail!(
                "WASM module did not finish within {} seconds",
                self.timeout.as_secs()
            ),
        }
    }

    async fn execute_unbounded(
        &self,
        module_path: &str,
        function_name: &str,
        input: Value,
    ) -> Result<Value> {
        let module_bytes = if module_path.starts_with("http://")
            || module_path.starts_with("https://")
        {
            let (client, url) = stormchaser_model::net_guard::guarded_client(module_path).await?;
            let res = client
                .get(url)
                .send()
                .await
                .with_context(|| format!("Failed to fetch WASM module from {}", module_path))?
                .error_for_status()
                .with_context(|| format!("Failed to fetch WASM module from {}", module_path))?;
            stormchaser_model::net_guard::read_body_capped(res, MAX_WASM_MODULE_BYTES)
                .await
                .with_context(|| format!("Failed to read bytes from {}", module_path))?
        } else if let Some(base64_data) = module_path.strip_prefix("base64://") {
            use base64::{engine::general_purpose, Engine as _};
            if base64_data.len() / 4 * 3 > MAX_WASM_MODULE_BYTES {
                anyhow::bail!(
                    "WASM module is larger than the {} byte limit",
                    MAX_WASM_MODULE_BYTES
                );
            }
            general_purpose::STANDARD
                .decode(base64_data)
                .with_context(|| "Failed to decode base64 WASM module")?
        } else {
            anyhow::bail!("Local file system access for WASM modules is disabled for security reasons. Use http(s):// or base64://");
        };

        // Compiling is CPU-bound and cannot yield, so it runs off the async executor.
        let engine = self.engine.clone();
        let module =
            tokio::task::spawn_blocking(move || Module::from_binary(&engine, &module_bytes))
                .await
                .context("WASM compilation task failed")??;
        let linker = Linker::new(&self.engine);

        // Add basic WASI if needed, but for now just pure WASM
        // wasmtime_wasi::add_to_linker(&mut linker, |s| s)?;

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.max_memory_bytes)
            .memories(1)
            .instances(1)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|limits| limits);
        // Fuel is not the limit here; it only makes the guest yield so the timeout can fire.
        store.set_fuel(u64::MAX)?;
        store.fuel_async_yield_interval(Some(FUEL_YIELD_INTERVAL))?;

        let instance = linker.instantiate_async(&mut store, &module).await?;

        let func = instance.get_typed_func::<(i32, i32), i32>(&mut store, function_name)?;

        // The interface is "JSON string in, JSON string out" through the module's exported
        // memory: `alloc` reserves space for the input, the function returns a pointer to the
        // result, and `get_result_len` gives its length.
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("WASM module must export 'memory'")?;

        let input_str = serde_json::to_string(&input)?;
        let input_bytes = input_str.as_bytes();
        let input_len =
            i32::try_from(input_bytes.len()).context("WASM input is too large to pass")?;

        // Find allocation functions in WASM
        let alloc = instance.get_typed_func::<i32, i32>(&mut store, "alloc")?;
        let dealloc = instance.get_typed_func::<(i32, i32), ()>(&mut store, "dealloc")?;

        let ptr = alloc.call_async(&mut store, input_len).await?;
        // Pointers are unsigned 32-bit offsets; an out-of-range one fails the bounds check.
        memory.write(&mut store, ptr as u32 as usize, input_bytes)?;

        let result_ptr = func.call_async(&mut store, (ptr, input_len)).await?;

        let get_len = instance.get_typed_func::<(), i32>(&mut store, "get_result_len")?;
        let result_len = get_len.call_async(&mut store, ()).await?;
        let result_len = usize::try_from(result_len)
            .ok()
            .filter(|len| *len <= MAX_WASM_RESULT_BYTES)
            .with_context(|| {
                format!(
                    "WASM module reported a result of {} bytes; the limit is {}",
                    result_len, MAX_WASM_RESULT_BYTES
                )
            })?;

        let mut result_bytes = vec![0u8; result_len];
        memory.read(&mut store, result_ptr as u32 as usize, &mut result_bytes)?;

        let result_str = String::from_utf8(result_bytes)?;
        let result_val: Value = serde_json::from_str(&result_str)?;

        // Cleanup
        dealloc.call_async(&mut store, (ptr, input_len)).await?;

        Ok(result_val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A module with the full interface whose `run` body is supplied by the test.
    fn module_with_run(memory: &str, run_body: &str, result_len: i64) -> String {
        let wat = format!(
            r#"
        (module
          (memory (export "memory") {memory})
          (func (export "alloc") (param $len i32) (result i32) (i32.const 1024))
          (func (export "dealloc") (param $ptr i32) (param $len i32))
          (func (export "get_result_len") (result i32) (i32.const {result_len}))
          (func (export "run") (param $ptr i32) (param $len i32) (result i32)
            {run_body}
          )
        )
        "#
        );
        let wasm_bytes = wat::parse_str(&wat).unwrap();
        use base64::{engine::general_purpose, Engine as _};
        format!("base64://{}", general_purpose::STANDARD.encode(&wasm_bytes))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_module_that_never_returns_is_stopped() {
        let module = module_with_run("1", "(loop $forever (br $forever)) (i32.const 0)", 0);
        let executor = WasmExecutor::with_limits(Duration::from_millis(300), 16 * 1024 * 1024);

        // On a single-threaded runtime this only returns if the guest yields.
        let started = std::time::Instant::now();
        let err = executor
            .execute(&module, "run", json!({}))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("did not finish"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn a_module_cannot_grow_memory_past_the_cap() {
        // Asks for 1024 more pages (64 MiB) and returns what memory.grow reported.
        let module = module_with_run(
            "1",
            "(if (i32.eq (memory.grow (i32.const 1024)) (i32.const -1)) (then (unreachable))) (i32.const 0)",
            0,
        );
        let capped = WasmExecutor::with_limits(Duration::from_secs(10), 16 * 1024 * 1024);
        let err = capped.execute(&module, "run", json!({})).await.unwrap_err();
        assert!(format!("{err:?}").contains("unreachable"), "{err:?}");

        // The same module gets past the grow when the cap allows it, and fails later on its
        // empty result instead.
        let roomy = WasmExecutor::with_limits(Duration::from_secs(10), 256 * 1024 * 1024);
        let err = roomy.execute(&module, "run", json!({})).await.unwrap_err();
        assert!(!format!("{err:?}").contains("unreachable"), "{err:?}");
    }

    #[tokio::test]
    async fn a_module_declaring_too_much_memory_is_not_instantiated() {
        let module = module_with_run("1024", "(i32.const 0)", 0);
        let executor = WasmExecutor::with_limits(Duration::from_secs(10), 16 * 1024 * 1024);
        let err = executor
            .execute(&module, "run", json!({}))
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("memory"), "{err:?}");
    }

    #[tokio::test]
    async fn an_oversized_or_negative_result_length_is_refused() {
        let executor = WasmExecutor::with_limits(Duration::from_secs(10), 16 * 1024 * 1024);
        for len in [-1, (MAX_WASM_RESULT_BYTES as i64) + 1] {
            let module = module_with_run("1", "(i32.const 0)", len);
            let err = executor
                .execute(&module, "run", json!({}))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("the limit is"), "{len}: {err}");
        }
    }

    #[tokio::test]
    async fn test_wasm_execution() -> Result<()> {
        // A simple WAT that echoes the input back with "echoed": true
        let wat = r#"
        (module
          (memory (export "memory") 1)
          (global $result_len (mut i32) (i32.const 0))
          (global $result_ptr (mut i32) (i32.const 0))

          (func (export "alloc") (param $len i32) (result i32)
            (i32.const 1024) ;; Simple static allocation for test
          )

          (func (export "dealloc") (param $ptr i32) (param $len i32)
            ;; No-op for test
          )

          (func (export "get_result_len") (result i32)
            (global.get $result_len)
          )

          (func (export "run") (param $ptr i32) (param $len i32) (result i32)
            ;; We just write a fixed JSON response for simplicity in this test
            (global.set $result_ptr (i32.const 2048))
            (global.set $result_len (i32.const 16))

            ;; Write '{"echoed": true}' to 2048
            (i32.store8 (i32.const 2048) (i32.const 123)) ;; {
            (i32.store8 (i32.const 2049) (i32.const 34))  ;; "
            (i32.store8 (i32.const 2050) (i32.const 101)) ;; e
            (i32.store8 (i32.const 2051) (i32.const 99))  ;; c
            (i32.store8 (i32.const 2052) (i32.const 104)) ;; h
            (i32.store8 (i32.const 2053) (i32.const 111)) ;; o
            (i32.store8 (i32.const 2054) (i32.const 101)) ;; e
            (i32.store8 (i32.const 2055) (i32.const 100)) ;; d
            (i32.store8 (i32.const 2056) (i32.const 34))  ;; "
            (i32.store8 (i32.const 2057) (i32.const 58))  ;; :
            (i32.store8 (i32.const 2058) (i32.const 32))  ;; space
            (i32.store8 (i32.const 2059) (i32.const 116)) ;; t
            (i32.store8 (i32.const 2060) (i32.const 114)) ;; r
            (i32.store8 (i32.const 2061) (i32.const 117)) ;; u
            (i32.store8 (i32.const 2062) (i32.const 101)) ;; e
            (i32.store8 (i32.const 2063) (i32.const 125)) ;; }

            (global.get $result_ptr)
          )
        )
        "#;

        let wasm_bytes = wat::parse_str(wat)?;
        use base64::{engine::general_purpose, Engine as _};
        let b64 = general_purpose::STANDARD.encode(&wasm_bytes);
        let module_path = format!("base64://{}", b64);

        let executor = WasmExecutor::new();
        let input = json!({"test": "data"});

        let result = executor.execute(&module_path, "run", input).await?;

        assert_eq!(result["echoed"], true);

        Ok(())
    }

    #[tokio::test]
    async fn test_wasm_execution_with_config() -> Result<()> {
        let wat = r#"
        (module
          (memory (export "memory") 1)
          (global $result_len (mut i32) (i32.const 0))
          (global $result_ptr (mut i32) (i32.const 0))

          (func (export "alloc") (param $len i32) (result i32)
            (i32.const 1024)
          )

          (func (export "dealloc") (param $ptr i32) (param $len i32)
          )

          (func (export "get_result_len") (result i32)
            (global.get $result_len)
          )

          (func (export "run") (param $ptr i32) (param $len i32) (result i32)
            (global.set $result_ptr (i32.const 2048))
            (global.set $result_len (i32.const 17))

            ;; Write '{"config": "val"}' to 2048
            (i32.store8 (i32.const 2048) (i32.const 123)) ;; {
            (i32.store8 (i32.const 2049) (i32.const 34))  ;; "
            (i32.store8 (i32.const 2050) (i32.const 99))  ;; c
            (i32.store8 (i32.const 2051) (i32.const 111)) ;; o
            (i32.store8 (i32.const 2052) (i32.const 110)) ;; n
            (i32.store8 (i32.const 2053) (i32.const 102)) ;; f
            (i32.store8 (i32.const 2054) (i32.const 105)) ;; i
            (i32.store8 (i32.const 2055) (i32.const 103)) ;; g
            (i32.store8 (i32.const 2056) (i32.const 34))  ;; "
            (i32.store8 (i32.const 2057) (i32.const 58))  ;; :
            (i32.store8 (i32.const 2058) (i32.const 32))  ;; space
            (i32.store8 (i32.const 2059) (i32.const 34))  ;; "
            (i32.store8 (i32.const 2060) (i32.const 118)) ;; v
            (i32.store8 (i32.const 2061) (i32.const 97))  ;; a
            (i32.store8 (i32.const 2062) (i32.const 108)) ;; l
            (i32.store8 (i32.const 2063) (i32.const 34))  ;; "
            (i32.store8 (i32.const 2064) (i32.const 125)) ;; }

            (global.get $result_ptr)
          )
        )
        "#;

        let wasm_bytes = wat::parse_str(wat)?;
        use base64::{engine::general_purpose, Engine as _};
        let b64 = general_purpose::STANDARD.encode(&wasm_bytes);
        let module_path = format!("base64://{}", b64);

        let executor = WasmExecutor::new();
        let input = json!({
            "spec": {},
            "params": {},
            "inputs": {},
            "config": {"key": "val"}
        });

        let result = executor.execute(&module_path, "run", input).await?;

        assert_eq!(result["config"], "val");

        Ok(())
    }
}
