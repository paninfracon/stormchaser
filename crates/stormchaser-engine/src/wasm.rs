use anyhow::{Context, Result};
use serde_json::Value;
use wasmtime::*;

pub struct WasmExecutor {
    engine: Engine,
}

impl Default for WasmExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmExecutor {
    pub fn new() -> Self {
        let config = Config::new();
        // config.async_support(true); // No longer needed in wasmtime 43.0
        let engine = Engine::new(&config).expect("Failed to create Wasmtime engine");
        Self { engine }
    }

    pub async fn execute(
        &self,
        module_path: &str,
        function_name: &str,
        input: Value,
    ) -> Result<Value> {
        let module_bytes = if module_path.starts_with("http://")
            || module_path.starts_with("https://")
        {
            reqwest::get(module_path)
                .await
                .with_context(|| format!("Failed to fetch WASM module from {}", module_path))?
                .bytes()
                .await
                .with_context(|| format!("Failed to read bytes from {}", module_path))?
                .to_vec()
        } else if let Some(base64_data) = module_path.strip_prefix("base64://") {
            use base64::{engine::general_purpose, Engine as _};
            general_purpose::STANDARD
                .decode(base64_data)
                .with_context(|| "Failed to decode base64 WASM module")?
        } else {
            anyhow::bail!("Local file system access for WASM modules is disabled for security reasons. Use http(s):// or base64://");
        };

        let module = Module::from_binary(&self.engine, &module_bytes)?;
        let linker = Linker::new(&self.engine);

        // Add basic WASI if needed, but for now just pure WASM
        // wasmtime_wasi::add_to_linker(&mut linker, |s| s)?;

        let mut store = Store::new(&self.engine, ());
        let instance = linker.instantiate_async(&mut store, &module).await?;

        let func = instance.get_typed_func::<(i32, i32), i32>(&mut store, function_name)?;

        // For simplicity, we assume the WASM module uses a shared memory and we can pass JSON.
        // This requires a more complex interaction (allocating in WASM memory).
        // For a prototype, let's assume a simpler interface if possible or just use string passing.

        // If the function takes no args and returns nothing, it's easy.
        // But we want to pass data.

        // Let's implement a simple "JSON string in, JSON string out" via exported memory.

        let memory = instance
            .get_memory(&mut store, "memory")
            .context("WASM module must export 'memory'")?;

        let input_str = serde_json::to_string(&input)?;
        let input_bytes = input_str.as_bytes();

        // Find allocation functions in WASM
        let alloc = instance.get_typed_func::<i32, i32>(&mut store, "alloc")?;
        let dealloc = instance.get_typed_func::<(i32, i32), ()>(&mut store, "dealloc")?;

        let ptr = alloc
            .call_async(&mut store, input_bytes.len() as i32)
            .await?;
        memory.write(&mut store, ptr as usize, input_bytes)?;

        let result_ptr_ptr = func
            .call_async(&mut store, (ptr, input_bytes.len() as i32))
            .await?;

        // Assume the function returns a pointer to a result structure or a pointer to the result string directly.
        // Let's assume it returns a pointer to a null-terminated string for now, or we can use another exported func to get length.

        // For this implementation, let's assume it returns a pointer to the start of the result string,
        // and we have a 'get_result_len' function.
        let get_len = instance.get_typed_func::<(), i32>(&mut store, "get_result_len")?;
        let result_len = get_len.call_async(&mut store, ()).await?;

        let mut result_bytes = vec![0u8; result_len as usize];
        memory.read(&mut store, result_ptr_ptr as usize, &mut result_bytes)?;

        let result_str = String::from_utf8(result_bytes)?;
        let result_val: Value = serde_json::from_str(&result_str)?;

        // Cleanup
        dealloc
            .call_async(&mut store, (ptr, input_bytes.len() as i32))
            .await?;

        Ok(result_val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
