use serde_json::json;
use stormchaser_model::{logging::LogBackend, StepId};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_log_pipeline_no_logs() {
    let mock_server = MockServer::start().await;

    let loki_response = json!({
        "status": "success",
        "data": {
            "resultType": "streams",
            "result": []
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(200).set_body_json(loki_response))
        .mount(&mock_server)
        .await;

    let backend = LogBackend::Loki {
        url: mock_server.uri(),
    };
    let step_id = StepId::new_v4();

    let logs = backend
        .fetch_step_logs("test", step_id, None, None, None)
        .await
        .unwrap();
    assert!(logs.is_empty(), "Logs should be empty");
}

#[tokio::test]
async fn test_log_pipeline_one_line() {
    let mock_server = MockServer::start().await;

    let loki_response = json!({
        "status": "success",
        "data": {
            "resultType": "streams",
            "result": [{
                "stream": { "job_name": "test" },
                "values": [
                    [ "1610000000000000000", "just one line" ]
                ]
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(200).set_body_json(loki_response))
        .mount(&mock_server)
        .await;

    let backend = LogBackend::Loki {
        url: mock_server.uri(),
    };
    let step_id = StepId::new_v4();

    let logs = backend
        .fetch_step_logs("test", step_id, None, None, None)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0], "just one line");
}

#[tokio::test]
async fn test_log_pipeline_exactly_page_boundary() {
    let mock_server = MockServer::start().await;

    // Generate exactly 10 lines
    let mut values = Vec::new();
    for i in 0..10 {
        values.push(json!([
            format!("161000000000000000{}", i),
            format!("line {}", i)
        ]));
    }

    let loki_response = json!({
        "status": "success",
        "data": {
            "resultType": "streams",
            "result": [{
                "stream": { "job_name": "test" },
                "values": values
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(200).set_body_json(loki_response))
        .mount(&mock_server)
        .await;

    let backend = LogBackend::Loki {
        url: mock_server.uri(),
    };
    let step_id = StepId::new_v4();

    let logs = backend
        .fetch_step_logs("test", step_id, None, None, Some(10))
        .await
        .unwrap();
    assert_eq!(logs.len(), 10);
    assert_eq!(logs[0], "line 0");
    assert_eq!(logs[9], "line 9");
}

#[tokio::test]
async fn test_log_pipeline_several_pages() {
    let mock_server = MockServer::start().await;

    // Mock should handle requests with different 'start' times correctly to simulate several pages.
    // If the backend doesn't implement pagination, it will just get the first page.
    // Let's assert that it gets 25 lines if limit is 10, meaning it should fetch 3 pages (10, 10, 5).

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use wiremock::{Request, Respond};

    struct PaginatedLoki {
        request_count: Arc<AtomicUsize>,
    }

    impl Respond for PaginatedLoki {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let count = self.request_count.fetch_add(1, Ordering::SeqCst);
            let mut values = Vec::new();

            // To simulate pagination properly:
            // The backend requests with limit=10.
            // First page returns 10.
            // Second page returns 10.
            // Third page returns 5.

            if count == 0 {
                for i in 0..10 {
                    values.push(json!([
                        format!("16100000000000000{:02}", i),
                        format!("line {}", i)
                    ]));
                }
            } else if count == 1 {
                // Must ensure the backend asks for start > last timestamp.
                // We'll just return the next 10 regardless to see if it makes 3 requests.
                for i in 10..20 {
                    values.push(json!([
                        format!("16100000000000000{:02}", i),
                        format!("line {}", i)
                    ]));
                }
            } else if count == 2 {
                for i in 20..25 {
                    values.push(json!([
                        format!("16100000000000000{:02}", i),
                        format!("line {}", i)
                    ]));
                }
            }

            let loki_response = json!({
                "status": "success",
                "data": {
                    "resultType": "streams",
                    "result": if values.is_empty() { vec![] } else { vec![json!({
                        "stream": { "job_name": "test" },
                        "values": values
                    })] }
                }
            });
            ResponseTemplate::new(200).set_body_json(loki_response)
        }
    }

    let request_count = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .respond_with(PaginatedLoki {
            request_count: request_count.clone(),
        })
        .mount(&mock_server)
        .await;

    let backend = LogBackend::Loki {
        url: mock_server.uri(),
    };
    let step_id = StepId::new_v4();

    // We pass None for limit here, because we want to test fetching ALL logs (which might be >5000 internally).
    // Or we can pass limit=25. But wait, if we pass limit=10 to the function, it implies we only WANT 10 logs.
    // Let's pass limit=None, but the mock returns pages of 10.
    // Wait, the client sends limit=5000 by default. If we want it to fetch pages of 10, the client needs a way to set the PAGE size.
    // The fetch_step_logs function doesn't expose page size. Let's just mock it returning 10 when the client asks for 5000.
    // A compliant Loki server will return fewer logs than requested if there's a hard server limit.

    let logs = backend
        .fetch_step_logs("test", step_id, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        request_count.load(Ordering::SeqCst),
        4,
        "Should have made 4 paginated requests (3 with data, 1 empty)"
    );
    assert_eq!(logs.len(), 25);
    assert_eq!(logs[0], "line 0");
    assert_eq!(logs[24], "line 24");
}

#[tokio::test]
async fn test_log_pipeline_embedded_newlines() {
    let mock_server = MockServer::start().await;

    let loki_response = json!({
        "status": "success",
        "data": {
            "resultType": "streams",
            "result": [{
                "stream": { "job_name": "test" },
                "values": [
                    [ "1610000000000000000", "line 1\r\nline 2\nline 3" ]
                ]
            }]
        }
    });

    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .respond_with(ResponseTemplate::new(200).set_body_json(loki_response))
        .mount(&mock_server)
        .await;

    let backend = LogBackend::Loki {
        url: mock_server.uri(),
    };
    let step_id = StepId::new_v4();

    let logs = backend
        .fetch_step_logs("test", step_id, None, None, None)
        .await
        .unwrap();

    // Some pipelines split by newline, some keep it as is.
    // The prompt says "Look specifically for missing lines, duplicate lines, lines with embedded '\r' or '\n' characters".
    // If it's expected to split them, it should be 3 lines.
    // If it doesn't split them, the TUI might look weird. We should split them at the backend level or handle it correctly.
    // Let's see what the backend actually does currently. Currently it just does `logs.push(log_line.to_string())`. It doesn't split.

    assert_eq!(
        logs.len(),
        3,
        "Embedded newlines should be split into multiple log lines"
    );
    assert_eq!(logs[0], "line 1");
    assert_eq!(logs[1], "line 2");
    assert_eq!(logs[2], "line 3");
}
