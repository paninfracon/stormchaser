use crate::api::{approve_step, delete_workflow_run, fetch_workflow_run_detail, reject_step};
use crate::runs::StepLogsPanel;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn RunDetailsPanel(
    run_id: String,
    #[prop(into)] selected_step_id: Signal<Option<String>>,
    set_selected_step_id: WriteSignal<Option<String>>,
    set_selected_run_id: WriteSignal<Option<String>>,
) -> impl IntoView {
    let run_id_clone = run_id.clone();
    let detail_resource = Resource::new(
        move || run_id_clone.clone(),
        |id| async move { fetch_workflow_run_detail(id).await },
    );

    // Step status SSE
    #[cfg(not(feature = "ssr"))]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;
        use web_sys::{EventSource, MessageEvent};

        let run_id_for_sse = run_id.clone();
        let _status_sse_effect = Effect::new(move |_| {
            if let Ok(es) =
                EventSource::new(&format!("/api/v1/runs/{}/status/stream", run_id_for_sse))
            {
                let es_clone = es.clone();
                let on_status = Closure::wrap(Box::new(move |event: MessageEvent| {
                    if let Some(_data) = event.data().as_string() {
                        detail_resource.refetch();
                    }
                }) as Box<dyn FnMut(MessageEvent)>);

                es.add_event_listener_with_callback(
                    "step_status",
                    on_status.as_ref().unchecked_ref(),
                )
                .unwrap();
                es.add_event_listener_with_callback(
                    "run_status",
                    on_status.as_ref().unchecked_ref(),
                )
                .unwrap();

                let ptr = Box::into_raw(Box::new((es_clone, on_status))) as usize;
                on_cleanup(move || {
                    let raw = ptr as *mut (EventSource, Closure<dyn FnMut(MessageEvent)>);
                    let boxed = unsafe { Box::from_raw(raw) };
                    boxed.0.close();
                });
            }
        });

        on_cleanup(move || {
            let _ = _status_sse_effect;
        });
    }

    // Update default step selection safely in an effect
    Effect::new(move |_| {
        if selected_step_id.get_untracked().is_none() {
            if let Some(Ok(detail)) = detail_resource.get() {
                if !detail.steps.is_empty() {
                    if let Some(id_val) = detail.steps[0].instance.get("id") {
                        if let Some(id_str) = id_val.as_str() {
                            set_selected_step_id.set(Some(id_str.to_string()));
                        }
                    }
                }
            }
        }
    });

    view! {
        <div style="display: flex; flex-direction: column; height: 100%;">
            <div style="display: flex; flex-direction: column; flex: 1; min-height: 0;">
                <div style="flex: none; max-height: 50%; overflow-y: auto; border-bottom: 1px solid var(--surface-border);">
                    <Transition fallback=|| view! { <div style="padding: 1.5rem;">"Loading details..."</div> }>
                        {move || match detail_resource.get() {
                            Some(Ok(detail)) => {
                                let steps = detail.steps.clone();
                                let active_run_id = detail.detail.id.to_string();
                                view! {
                                    <div style="padding: 1.5rem;">
                                        <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                                            <h3 style="margin: 0 0 0.5rem 0;">{detail.detail.workflow_name.clone()}</h3>
                                            {
                                                let delete_run_id = active_run_id.clone();
                                                view! {
                                                    <button
                                                        class="btn btn-secondary"
                                                        style="color: var(--status-error); border-color: var(--status-error); padding: 0.25rem 0.75rem; font-size: 0.85rem;"
                                                        on:click=move |_| {
                                                            let r_id = delete_run_id.clone();
                                                            spawn_local(async move {
                                                                if delete_workflow_run(r_id).await.is_ok() {
                                                                    set_selected_step_id.set(None);
                                                                    set_selected_run_id.set(None);
                                                                }
                                                            });
                                                        }
                                                    >
                                                        "Delete Run"
                                                    </button>
                                                }
                                            }
                                        </div>
                                    <div style="display: flex; flex-direction: column; gap: 0.25rem; color: var(--text-secondary); font-size: 0.9rem; margin-bottom: 1.5rem;">
                                        <span>"Run ID: " {active_run_id.clone()}</span>
                                        <span>"Status: " {String::from(detail.detail.status.clone())}</span>
                                        <span>"Initiator: " {detail.detail.initiating_user.clone()}</span>
                                        <span>"Created: " {detail.detail.created_at.format("%Y-%m-%d %H:%M:%S").to_string()}</span>
                                        {detail.detail.finished_at.map(|t| view! { <span>"Finished: " {t.format("%Y-%m-%d %H:%M:%S").to_string()}</span> }.into_any()).unwrap_or_else(|| ().into_any())}
                                    </div>

                                    <h4 style="margin: 0 0 1rem 0;">"Steps"</h4>
                                    <ul style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.5rem;">
                                        {steps.clone().into_iter().map(move |step| {
                                            let step_id = step.instance.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            let step_name = step.instance.get("step_name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
                                            let status_str = step.instance.get("status").and_then(|v| v.as_str()).unwrap_or("pending");

                                            let status_class = match status_str {
                                                "initializing" => "status-initializing",
                                                "running" => "status-running",
                                                "unpacking_sfs" | "packing_sfs" => "status-running",
                                                "succeeded" => "status-succeeded",
                                                "failed" => "status-failed",
                                                _ => "status-queued",
                                            };

                                            let is_selected = selected_step_id.get() == Some(step_id.clone());
                                            let step_id_clone = step_id.clone();

                                            view! {
                                                <li
                                                    style=move || format!("padding: 1rem; cursor: pointer; border: 1px solid var(--surface-border); border-radius: 8px; {}", if is_selected { "background: var(--surface-elevated); border-color: var(--primary-color);" } else { "" })
                                                    on:click=move |_| set_selected_step_id.set(Some(step_id_clone.clone()))
                                                >
                                                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
                                                        <div style="font-weight: 500; font-size: 1rem;">{step_name}</div>
                                                        <span class=format!("status-badge {}", status_class) style="font-size: 0.75rem;">
                                                            {status_str.to_string()}
                                                        </span>
                                                    </div>

                                                    {if status_str == "waiting_for_event" && is_selected {
                                                        let s_run_id = active_run_id.clone();
                                                        let s_step_id = step_id_clone.clone();
                                                        let on_approve = move |_| {
                                                            let r_id = s_run_id.clone();
                                                            let s_id = s_step_id.clone();
                                                            spawn_local(async move {
                                                                if approve_step(r_id, s_id, serde_json::json!({})).await.is_ok() {
                                                                    detail_resource.refetch();
                                                                }
                                                            });
                                                        };
                                                        let sr_run_id = active_run_id.clone();
                                                        let sr_step_id = step_id_clone.clone();
                                                        let on_reject = move |_| {
                                                            let r_id = sr_run_id.clone();
                                                            let s_id = sr_step_id.clone();
                                                            spawn_local(async move {
                                                                if reject_step(r_id, s_id).await.is_ok() {
                                                                    detail_resource.refetch();
                                                                }
                                                            });
                                                        };
                                                        view! {
                                                            <div style="margin-top: 1rem; display: flex; gap: 0.5rem;">
                                                                <button class="btn btn-primary" on:click=on_approve>"Approve"</button>
                                                                <button class="btn btn-secondary" style="color: var(--status-error); border-color: var(--status-error);" on:click=on_reject>"Reject"</button>
                                                            </div>
                                                        }.into_any()
                                                    } else {
                                                        ().into_any()
                                                    }}

                                                    {if is_selected {
                                                        let started = step.instance.get("started_at").and_then(|v| v.as_str()).unwrap_or("-").to_string();
                                                        let finished = step.instance.get("finished_at").and_then(|v| v.as_str()).unwrap_or("-").to_string();

                                                        view! {
                                                            <div style="margin-top: 1rem; font-size: 0.85rem; color: var(--text-secondary); display: flex; flex-direction: column; gap: 0.5rem; border-top: 1px solid var(--surface-border); padding-top: 1rem;">
                                                                <div>
                                                                    <strong>"Timing:"</strong><br/>
                                                                    "Started: " {started}<br/>
                                                                    "Finished: " {finished}
                                                                </div>

                                                                {if !step.outputs.is_empty() {
                                                                    view! {
                                                                        <div>
                                                                            <strong>"Outputs:"</strong>
                                                                            <ul style="margin: 0.25rem 0 0 1rem; padding: 0;">
                                                                                {step.outputs.into_iter().map(|out| {
                                                                                    let key = out.get("key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                                                    let is_sensitive = out.get("is_sensitive").and_then(|v| v.as_bool()).unwrap_or(false);
                                                                                    let val = if is_sensitive {
                                                                                        "*** REDACTED ***".to_string()
                                                                                    } else if let Some(v) = out.get("value") {
                                                                                        if let Some(s) = v.as_str() { s.to_string() } else { v.to_string() }
                                                                                    } else {
                                                                                        "null".to_string()
                                                                                    };
                                                                                    view! { <li style="margin-bottom: 0.25rem;"><code>{key}</code>": " {val}</li> }
                                                                                }).collect_view()}
                                                                            </ul>
                                                                        </div>
                                                                    }.into_any()
                                                                } else {
                                                                    ().into_any()
                                                                }}

                                                                {if !step.history.is_empty() {
                                                                    view! {
                                                                        <div style="margin-top: 1rem;">
                                                                            <strong>"History:"</strong>
                                                                            <table style="width: 100%; border-collapse: collapse; margin-top: 0.5rem; font-size: 0.85rem;">
                                                                                <thead>
                                                                                    <tr style="border-bottom: 1px solid var(--surface-border); text-align: left; color: var(--text-secondary);">
                                                                                        <th style="padding: 0.25rem 0.5rem; font-weight: 500;">"Status"</th>
                                                                                        <th style="padding: 0.25rem 0.5rem; font-weight: 500;">"Time"</th>
                                                                                    </tr>
                                                                                </thead>
                                                                                <tbody>
                                                                                    {step.history.into_iter().map(|hist| {
                                                                                        let h_status = hist.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                                                        let h_time = hist.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                                                        view! {
                                                                                            <tr style="border-bottom: 1px solid var(--surface-border);">
                                                                                                <td style="padding: 0.25rem 0.5rem;">{h_status}</td>
                                                                                                <td style="padding: 0.25rem 0.5rem; color: var(--text-secondary);">{h_time}</td>
                                                                                            </tr>
                                                                                        }
                                                                                    }).collect_view()}
                                                                                </tbody>
                                                                            </table>
                                                                        </div>
                                                                    }.into_any()
                                                                } else {
                                                                    ().into_any()
                                                                }}
                                                            </div>
                                                        }.into_any()
                                                    } else {
                                                        ().into_any()
                                                    }}
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ul>

                                    {if !detail.artifacts.is_empty() {
                                        view! {
                                            <h4 style="margin: 1.5rem 0 1rem 0;">"Artifacts"</h4>
                                            <ul style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.5rem; font-size: 0.9rem;">
                                                {detail.artifacts.clone().into_iter().map(|artifact| {
                                                    let name = artifact.get("artifact_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                    view! {
                                                        <li style="padding: 0.75rem; border: 1px solid var(--surface-border); border-radius: 6px; background: var(--surface-default);">
                                                            <strong>{name}</strong>
                                                        </li>
                                                    }
                                                }).collect_view()}
                                            </ul>
                                        }.into_any()
                                    } else {
                                        ().into_any()
                                    }}

                                    {if !detail.test_cases.is_empty() {
                                        let passed = detail.test_cases.iter().filter(|t| t.get("status").and_then(|v| v.as_str()) == Some("passed")).count();
                                        let failed = detail.test_cases.iter().filter(|t| t.get("status").and_then(|v| v.as_str()) == Some("failed")).count();
                                        let errors = detail.test_cases.iter().filter(|t| t.get("status").and_then(|v| v.as_str()) == Some("error")).count();
                                        view! {
                                            <h4 style="margin: 1.5rem 0 1rem 0;">"Test Reports"</h4>
                                            <div style="margin-bottom: 1rem; font-size: 0.9rem; color: var(--text-secondary);">
                                                "Passed: " {passed} " | Failed: " <span style="color: var(--status-error);">{failed}</span> " | Errors: " <span style="color: var(--status-error);">{errors}</span>
                                            </div>
                                            <div style="display: flex; flex-wrap: wrap; gap: 0.25rem;">
                                                {detail.test_cases.clone().into_iter().map(|tc| {
                                                    let status = tc.get("status").and_then(|v| v.as_str()).unwrap_or("skipped");
                                                    let color = match status {
                                                        "passed" => "var(--status-success)",
                                                        "failed" | "error" => "var(--status-error)",
                                                        _ => "var(--status-queued)",
                                                    };
                                                    let symbol = match status {
                                                        "passed" => "✔",
                                                        "failed" => "✘",
                                                        "error" => "!",
                                                        _ => "○",
                                                    };
                                                    let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                    view! {
                                                        <span style=format!("color: {}; font-weight: bold; cursor: pointer;", color) title=name>{symbol}</span>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }.into_any()
                                    } else {
                                        ().into_any()
                                    }}
                                    </div>
                        }.into_any()
                    },
                    Some(Err(e)) => view! { <div style="padding: 1.5rem; color: var(--status-error);">"Error loading details: " {e.to_string()}</div> }.into_any(),
                    None => view! { <div></div> }.into_any()
                }}
                    </Transition>
                </div>
                <div style="flex: 1; display: flex; flex-direction: column; min-height: 0;">
                    <StepLogsPanel run_id=run_id.clone() step_id=selected_step_id />
                </div>
            </div>
        </div>
    }
}
