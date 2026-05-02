use k8s_openapi::api::core::v1::Pod;

pub fn do_extract_pod_metrics_with_reason(pods: Vec<Pod>) -> (Option<i32>, i32, Option<String>) {
    let mut max_restart_count = 0;
    let mut exit_code = None;
    let mut failure_reason = None;

    for pod in pods {
        if let Some(status) = pod.status {
            if let Some(container_statuses) = status.container_statuses {
                for cs in container_statuses {
                    max_restart_count = max_restart_count.max(cs.restart_count);
                    if let Some(state) = cs.state {
                        if let Some(terminated) = state.terminated {
                            exit_code = Some(terminated.exit_code);
                            if terminated.exit_code != 0 {
                                let reason = terminated.reason.as_deref().unwrap_or("Terminated");
                                let message = terminated.message.as_deref().unwrap_or("");
                                failure_reason = Some(if message.is_empty() {
                                    reason.to_string()
                                } else {
                                    format!("{}: {}", reason, message)
                                });
                            }
                        } else if let Some(waiting) = state.waiting {
                            let reason = waiting.reason.as_deref().unwrap_or("Waiting");
                            let message = waiting.message.as_deref().unwrap_or("");
                            failure_reason = Some(if message.is_empty() {
                                reason.to_string()
                            } else {
                                format!("{}: {}", reason, message)
                            });
                        }
                    }
                }
            }
        }
    }

    (exit_code, 1 + max_restart_count, failure_reason)
}
