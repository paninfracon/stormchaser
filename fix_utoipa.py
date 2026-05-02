import re
import os

routes = [
    {
        "file": "crates/stormchaser-api/src/routes/workflow.rs",
        "fns": [
            ("delete_workflow_run_api", 'delete', '"/api/v1/runs/{run_id}"', '("run_id" = Uuid, Path, description="Run ID")', 'workflow'),
        ]
    },
    {
        "file": "crates/stormchaser-api/src/routes/cron.rs",
        "fns": [
            ("create_cron_workflow", 'post', '"/api/v1/cron"', '', 'cron'),
            ("list_cron_workflows", 'get', '"/api/v1/cron"', '', 'cron'),
            ("delete_cron_workflow", 'delete', '"/api/v1/cron/{id}"', '("id" = Uuid, Path, description="Cron ID")', 'cron'),
        ]
    },
    {
        "file": "crates/stormchaser-api/src/routes/storage.rs",
        "fns": [
            ("create_storage_backend", 'post', '"/api/v1/storage/backends"', '', 'storage'),
            ("list_storage_backends", 'get', '"/api/v1/storage/backends"', '', 'storage'),
            ("get_storage_backend", 'get', '"/api/v1/storage/backends/{id}"', '("id" = Uuid, Path, description="Backend ID")', 'storage'),
            ("update_storage_backend", 'put', '"/api/v1/storage/backends/{id}"', '("id" = Uuid, Path, description="Backend ID")', 'storage'),
            ("delete_storage_backend", 'delete', '"/api/v1/storage/backends/{id}"', '("id" = Uuid, Path, description="Backend ID")', 'storage'),
            ("list_run_test_reports", 'get', '"/api/v1/runs/{run_id}/reports"', '("run_id" = Uuid, Path, description="Run ID")', 'storage'),
            ("list_run_test_summaries", 'get', '"/api/v1/runs/{run_id}/test-summaries"', '("run_id" = Uuid, Path, description="Run ID")', 'storage'),
        ]
    },
    {
        "file": "crates/stormchaser-api/src/routes/webhook.rs",
        "fns": [
            ("create_webhook", 'post', '"/api/v1/webhooks"', '', 'webhook'),
            ("list_webhooks", 'get', '"/api/v1/webhooks"', '', 'webhook'),
            ("get_webhook", 'get', '"/api/v1/webhooks/{id}"', '("id" = Uuid, Path, description="Webhook ID")', 'webhook'),
            ("delete_webhook", 'delete', '"/api/v1/webhooks/{id}"', '("id" = Uuid, Path, description="Webhook ID")', 'webhook'),
            ("create_event_rule", 'post', '"/api/v1/rules"', '', 'webhook'),
            ("list_event_rules", 'get', '"/api/v1/rules"', '', 'webhook'),
        ]
    },
    {
        "file": "crates/stormchaser-api/src/routes/step.rs",
        "fns": [
            ("stream_step_logs_api", 'get', '"/api/v1/runs/{run_id}/steps/{step_id}/logs/stream"', '("run_id" = Uuid, Path, description="Run ID"), ("step_id" = String, Path, description="Step ID")', 'step'),
            ("stream_run_logs_api", 'get', '"/api/v1/runs/{run_id}/logs/stream"', '("run_id" = Uuid, Path, description="Run ID")', 'step'),
        ]
    }
]

for r in routes:
    with open(r["file"], "r") as f:
        content = f.read()

    for fn, method, path, params, tag in r["fns"]:
        if f"pub async fn {fn}" in content and f"__path_{fn}" not in content:
            # Check if it already has utoipa::path
            fn_idx = content.find(f"pub async fn {fn}")
            if content.rfind("#[utoipa::path", 0, fn_idx) < fn_idx - 200:
                params_str = f"params({params}),\n    " if params else ""
                macro = f"""#[utoipa::path(
    {method},
    path = {path},
    {params_str}responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "{tag}"
)]
"""
                content = content[:fn_idx] + macro + content[fn_idx:]

    with open(r["file"], "w") as f:
        f.write(content)
