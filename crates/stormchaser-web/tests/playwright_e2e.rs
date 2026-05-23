use std::io::{BufRead, BufReader};
use std::process::Stdio;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_playwright_tests() {
    println!("Building WASM frontend...");
    let mut build_cmd = std::process::Command::new("cargo");
    let status = build_cmd
        .arg("leptos")
        .arg("build")
        .arg("--project")
        .arg("stormchaser-web")
        .status()
        .expect("Failed to build Leptos project");
    assert!(status.success(), "cargo leptos build failed");

    let mock_server = MockServer::start().await;

    // Provide default mocks for the endpoints called by layout / pages
    Mock::given(method("GET"))
        .and(path("/api/v1/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/connections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/webhooks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/rules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/cron-workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let workspace_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let site_root = format!("{}/../../target/site", workspace_root);

    println!("Starting web server...");
    let mut web_server_cmd = std::process::Command::new("cargo");
    web_server_cmd
        .arg("run")
        .arg("-p")
        .arg("stormchaser-web")
        .arg("--features=ssr")
        .env("API_URL", mock_server.uri())
        .env("EXTERNAL_API_URL", mock_server.uri())
        .env("WEB_URL", "http://127.0.0.1:3009")
        .env("LEPTOS_SITE_ADDR", "127.0.0.1:3009")
        .env("LEPTOS_OUTPUT_NAME", "stormchaser-web")
        .env("LEPTOS_SITE_ROOT", &site_root)
        // explicitly set the CWD to the workspace root, because cargo run -p runs from workspace root in cargo leptos
        .current_dir(format!("{}/../..", workspace_root))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut web_server = web_server_cmd
        .spawn()
        .expect("Failed to start stormchaser-web");
    let stdout = web_server.stdout.take().expect("Failed to open stdout");

    // Wait for the axum server to bind by reading stdout
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut server_ready = false;

    // Read lines until we see "listening on"
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                print!("SERVER: {}", line);
                if line.contains("listening on") {
                    server_ready = true;
                    break;
                }
            }
            Err(_) => break,
        }
    }

    if !server_ready {
        let _ = web_server.kill();
        panic!("Server failed to start or didn't output 'listening on'");
    }

    // Server is ready, spawn a thread to consume the rest of stdout so it doesn't block
    std::thread::spawn(move || loop {
        let mut l = String::new();
        if reader.read_line(&mut l).unwrap_or(0) == 0 {
            break;
        }
    });

    println!("Running Playwright tests...");
    let mut cmd = tokio::process::Command::new("npx");
    cmd.arg("playwright")
        .arg("test")
        .current_dir("e2e")
        .env("WEB_URL", "http://127.0.0.1:3009")
        .env("CI", "1")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR");

    let status = cmd
        .status()
        .await
        .expect("Failed to execute npx playwright test");

    // Kill the web server
    let _ = web_server.kill();
    let _ = web_server.wait();

    assert!(status.success(), "Playwright tests failed");
}
