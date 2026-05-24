use crate::api::{fetch_workflow_run_detail, fetch_workflow_runs};
use crate::models::{RunStatus, WorkflowRunDetail};
use crate::run_modal::CreateRunModal;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn WorkflowRunsList() -> impl IntoView {
    let (filter_owner, set_filter_owner) = signal(None::<String>);
    let (filter_name, set_filter_name) = signal(None::<String>);
    let (filter_repo_url, set_filter_repo_url) = signal(None::<String>);
    let (filter_workflow_path, set_filter_workflow_path) = signal(None::<String>);
    let (filter_created_after, set_filter_created_after) = signal(None::<String>);
    let (filter_created_before, set_filter_created_before) = signal(None::<String>);
    let (filter_status, set_filter_status) = signal(Some("Any".to_string()));

    // We use a Resource to fetch the workflow runs from the server function.
    let runs_resource = Resource::new(
        move || {
            (
                filter_status.get(),
                filter_owner.get(),
                filter_name.get(),
                filter_repo_url.get(),
                filter_workflow_path.get(),
                filter_created_after.get(),
                filter_created_before.get(),
            )
        },
        |(status, owner, name, repo, path, after, before)| async move {
            let status_val = if status.as_deref() == Some("Any") {
                None
            } else {
                status
            };
            fetch_workflow_runs(status_val, owner, name, repo, path, after, before).await
        },
    );

    let (selected_run_id, set_selected_run_id) = signal(None::<String>);
    let (selected_step_id, set_selected_step_id) = signal(None::<String>);
    let (show_create_modal, set_show_create_modal) = signal(false);

    // Clear step selection when run changes
    let _run_change_effect = Effect::new(move |_| {
        selected_run_id.get(); // Track run id changes
        set_selected_step_id.set(None);
    });

    // SSE EventSource for real-time updates
    #[cfg(not(feature = "ssr"))]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;
        use web_sys::{EventSource, MessageEvent};

        let is_authed = Memo::new(move |_| matches!(runs_resource.get(), Some(Ok(_))));

        let _sse_effect = Effect::new(move |_| {
            web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
                "SSE Effect triggered! is_authed: {}",
                is_authed.get()
            )));

            if is_authed.get() {
                web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                    "Attempting to connect EventSource...",
                ));
                if let Ok(es) = EventSource::new("/api/v1/runs/stream") {
                    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                        "EventSource created successfully",
                    ));
                    let es_clone = es.clone();
                    let on_message =
                        Closure::wrap(Box::new(move |event: MessageEvent| {
                            if let Some(data) = event.data().as_string() {
                                web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
                                    &format!("Received SSE data: {}", data),
                                ));
                                runs_resource.refetch();
                            }
                        }) as Box<dyn FnMut(MessageEvent)>);

                    es.add_event_listener_with_callback(
                        "workflow_run",
                        on_message.as_ref().unchecked_ref(),
                    )
                    .unwrap();

                    let ptr = Box::into_raw(Box::new((es_clone, on_message))) as usize;
                    on_cleanup(move || {
                        let raw = ptr as *mut (EventSource, Closure<dyn FnMut(MessageEvent)>);
                        let boxed = unsafe { Box::from_raw(raw) };
                        boxed.0.close();
                    });
                }
            }
        });

        on_cleanup(move || {
            let _ = _sse_effect;
        });
    }

    view! {

        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            {move || if show_create_modal.get() {
                view! {
                    <CreateRunModal
                        on_close=Callback::new(move |_| set_show_create_modal.set(false))
                        on_success=Callback::new(move |_| {
                            set_show_create_modal.set(false);
                            runs_resource.refetch();
                        })
                    />
                }.into_any()
            } else { ().into_any() }}

            <div class="glass-panel table-container" style=move || if selected_run_id.get().is_some() { "flex: 1; overflow: hidden; max-width: 50%; display: flex; flex-direction: column;" } else { "flex: 1; overflow: hidden; display: flex; flex-direction: column;" }>
                <div style="display: flex; justify-content: space-between; align-items: center; padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem;">"Workflow Runs"</h2>
                    <button
                        style="padding: 0.5rem 1rem; border-radius: 4px; border: none; background: var(--primary-color); color: white; cursor: pointer; font-weight: 500;"
                        on:click=move |_| set_show_create_modal.set(true)
                    >
                        "Create Run"
                    </button>
                </div>
                <RunsFilterPanel
                        set_filter_owner=set_filter_owner
                        set_filter_name=set_filter_name
                        set_filter_repo_url=set_filter_repo_url
                        set_filter_workflow_path=set_filter_workflow_path
                        set_filter_created_after=set_filter_created_after
                        set_filter_created_before=set_filter_created_before
                        set_filter_status=set_filter_status
                        current_status=filter_status
                        current_owner=filter_owner
                        current_name=filter_name
                        current_repo_url=filter_repo_url
                        current_workflow_path=filter_workflow_path
                        current_created_after=filter_created_after
                        current_created_before=filter_created_before
                    />
                    <div style="flex: 1; overflow: auto;">
                    <Suspense fallback=move || view! { <div style="padding: 2rem; text-align: center;">"Loading runs..."</div> }>
                        {
                            let runs_list = Memo::new(move |_| {
                                match runs_resource.get() {
                                    Some(Ok(runs)) => runs,
                                    _ => vec![],
                                }
                            });

                            move || match runs_resource.get() {
                            Some(Ok(runs)) => {
                                if runs.is_empty() {
                                    view! {
                                        <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">
                                            "No workflow runs found."
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <RunsTable runs=runs_list selected_run_id=selected_run_id set_selected_run_id=set_selected_run_id /> }.into_any()
                                }
                            }
                            Some(Err(e)) => {
                                // If unauthorized, we could show a message to login.
                                let err_msg = e.to_string();
                                let is_unauth = err_msg.contains("Unauthorized");
                                view! {
                                    <div style="padding: 2rem; text-align: center; color: var(--status-error);">
                                        {if is_unauth {
                                            "Please login to view workflow runs.".to_string()
                                        } else {
                                            format!("Failed to load runs: {}", err_msg)
                                        }}
                                    </div>
                                }.into_any()
                            }
                            None => ().into_any()
                        }}
                    </Suspense>
                    </div>
                </div>

                {move || selected_run_id.get().map(|run_id| {
                    view! {
                        <div class="glass-panel" style="flex: 1; display: flex; flex-direction: column; overflow: hidden;">
                            <RunDetailsPanel run_id=run_id selected_step_id=selected_step_id set_selected_step_id=set_selected_step_id />
                        </div>
                    }
                })}
            </div>
    }
}

#[component]
fn RunDetailsPanel(
    run_id: String,
    #[prop(into)] selected_step_id: Signal<Option<String>>,
    set_selected_step_id: WriteSignal<Option<String>>,
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
                                        <h3 style="margin: 0 0 0.5rem 0;">{detail.detail.workflow_name.clone()}</h3>
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
                                                "running" => "status-running",
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

#[component]
fn StepLogsPanel(run_id: String, step_id: Signal<Option<String>>) -> impl IntoView {
    let run_id_clone = run_id.clone();
    #[cfg(not(feature = "ssr"))]
    let run_id_for_sse = run_id.clone();

    let logs_signal = RwSignal::new(None::<Vec<String>>);

    let _fetch_effect = Effect::new(move |_| {
        if let Some(sid) = step_id.get() {
            let rid = run_id_clone.clone();
            spawn_local(async move {
                logs_signal.set(None);
                if let Ok(initial_logs) = crate::api::fetch_step_logs(rid, sid, Some(100)).await {
                    logs_signal.set(Some(initial_logs));
                } else {
                    logs_signal.set(Some(Vec::new()));
                }
            });
        } else {
            logs_signal.set(Some(Vec::new()));
        }
    });

    #[cfg(not(feature = "ssr"))]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;
        use web_sys::{EventSource, MessageEvent};

        let _logs_sse_effect = Effect::new(move |_| {
            if let Some(sid) = step_id.get() {
                let rid = run_id_for_sse.clone();
                if let Ok(es) =
                    EventSource::new(&format!("/api/v1/runs/{}/steps/{}/logs/stream", rid, sid))
                {
                    let es_clone = es.clone();
                    let on_log = Closure::wrap(Box::new(move |event: MessageEvent| {
                        if let Some(data) = event.data().as_string() {
                            logs_signal.update(|v| {
                                if let Some(list) = v {
                                    list.push(data);
                                } else {
                                    *v = Some(vec![data]);
                                }
                            });

                            // Force a scroll to bottom
                            if let Some(window) = web_sys::window() {
                                if let Some(doc) = window.document() {
                                    if let Some(el) = doc.get_element_by_id("logs-container") {
                                        el.set_scroll_top(el.scroll_height());
                                    }
                                }
                            }
                        }
                    })
                        as Box<dyn FnMut(MessageEvent)>);

                    es.add_event_listener_with_callback("log", on_log.as_ref().unchecked_ref())
                        .unwrap();

                    let ptr = Box::into_raw(Box::new((es_clone, on_log))) as usize;
                    on_cleanup(move || {
                        let raw = ptr as *mut (EventSource, Closure<dyn FnMut(MessageEvent)>);
                        let boxed = unsafe { Box::from_raw(raw) };
                        boxed.0.close();
                    });
                }
            }
        });

        on_cleanup(move || {
            let _ = _logs_sse_effect;
        });
    }

    view! {
        <div style="flex: 1; display: flex; flex-direction: column; background: #1e1e1e; color: #d4d4d4; font-family: monospace; overflow: hidden; height: 100%;">
            <div style="padding: 0.5rem 1rem; background: #2d2d2d; border-bottom: 1px solid #404040; font-size: 0.85rem; display: flex; justify-content: space-between;">
                <span>"Logs"</span>
            </div>
            <div id="logs-container" style="flex: 1; overflow-y: auto; padding: 1rem; font-size: 0.85rem; line-height: 1.5; white-space: pre-wrap; word-break: break-all;">
                {move || {
                    let sid_opt = step_id.get();
                    if sid_opt.is_none() {
                        return view! { <div style="color: #808080;">"Select a step to view logs"</div> }.into_any();
                    }

                    let logs_opt = logs_signal.get();

                    match logs_opt {
                        None => view! { <div style="color: #808080;">"Loading logs..."</div> }.into_any(),
                        Some(logs) if logs.is_empty() => view! { <div style="color: #808080;">"No logs available for this step."</div> }.into_any(),
                        Some(logs) => {
                            let content = logs.join("\n");
                            view! { <div>{content}</div> }.into_any()
                        }
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn RunsTable(
    #[prop(into)] runs: Signal<Vec<WorkflowRunDetail>>,
    #[prop(into)] selected_run_id: Signal<Option<String>>,
    set_selected_run_id: WriteSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <table>
            <thead>
                <tr>
                    <th>"Name"</th>
                    <th>"Initiator"</th>
                    <th>"Status"</th>
                    <th>"Created"</th>
                    <th>"Finished"</th>
                </tr>
            </thead>
            <tbody>
                {move || runs.get().into_iter().map(|run| {
                    let status_class = match run.status {
                        RunStatus::Queued => "status-queued",
                        RunStatus::Resolving => "status-resolving",
                        RunStatus::StartPending => "status-start_pending",
                        RunStatus::Running => "status-running",
                        RunStatus::Succeeded => "status-succeeded",
                        RunStatus::Failed => "status-failed",
                        RunStatus::Aborted => "status-aborted",
                    };
                    let status_text = String::from(run.status.clone());

                    view! {
                        <tr
                            style=move || if selected_run_id.get() == Some(run.id.to_string()) { "cursor: pointer; background: var(--surface-elevated);" } else { "cursor: pointer;" }
                            on:click=move |_| {
                                let id_str = run.id.to_string();
                                set_selected_run_id.update(move |current| {
                                    if current.as_deref() == Some(&id_str) {
                                        *current = None; // Deselect on double click
                                    } else {
                                        *current = Some(id_str.clone());
                                    }
                                });
                            }
                        >
                            <td style="font-weight: 500;">{run.workflow_name.clone()}</td>
                            <td style="color: var(--text-secondary);">{run.initiating_user.clone()}</td>
                            <td>
                                <span class=format!("status-badge {}", status_class)>
                                    {status_text}
                                </span>
                            </td>
                            <td style="color: var(--text-secondary);">
                                {run.created_at.format("%Y-%m-%d %H:%M:%S").to_string()}
                            </td>
                            <td style="color: var(--text-secondary);">
                                {run.finished_at.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(|| "-".to_string())}
                            </td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
}

#[component]
fn RunsFilterPanel(
    set_filter_owner: WriteSignal<Option<String>>,
    set_filter_name: WriteSignal<Option<String>>,
    set_filter_repo_url: WriteSignal<Option<String>>,
    set_filter_workflow_path: WriteSignal<Option<String>>,
    set_filter_created_after: WriteSignal<Option<String>>,
    set_filter_created_before: WriteSignal<Option<String>>,
    set_filter_status: WriteSignal<Option<String>>,
    #[prop(into)] current_status: Signal<Option<String>>,
    #[prop(into)] current_owner: Signal<Option<String>>,
    #[prop(into)] current_name: Signal<Option<String>>,
    #[prop(into)] current_repo_url: Signal<Option<String>>,
    #[prop(into)] current_workflow_path: Signal<Option<String>>,
    #[prop(into)] current_created_after: Signal<Option<String>>,
    #[prop(into)] current_created_before: Signal<Option<String>>,
) -> impl IntoView {
    let (is_expanded, set_is_expanded) = signal(false);

    // Local state for inputs
    let (local_owner, set_local_owner) = signal(current_owner.get_untracked().unwrap_or_default());
    let (local_name, set_local_name) = signal(current_name.get_untracked().unwrap_or_default());
    let (local_repo_url, set_local_repo_url) =
        signal(current_repo_url.get_untracked().unwrap_or_default());
    let (local_workflow_path, set_local_workflow_path) =
        signal(current_workflow_path.get_untracked().unwrap_or_default());
    let (local_created_after, set_local_created_after) =
        signal(current_created_after.get_untracked().unwrap_or_default());
    let (local_created_before, set_local_created_before) =
        signal(current_created_before.get_untracked().unwrap_or_default());
    let (local_status, set_local_status) = signal(
        current_status
            .get_untracked()
            .unwrap_or_else(|| "Any".to_string()),
    );

    let apply_filters = move |_| {
        let o = local_owner.get();
        let n = local_name.get();
        let r = local_repo_url.get();
        let w = local_workflow_path.get();
        let ca = local_created_after.get();
        let cb = local_created_before.get();
        let s = local_status.get();

        set_filter_owner.set(if o.is_empty() { None } else { Some(o) });
        set_filter_name.set(if n.is_empty() { None } else { Some(n) });
        set_filter_repo_url.set(if r.is_empty() { None } else { Some(r) });
        set_filter_workflow_path.set(if w.is_empty() { None } else { Some(w) });
        set_filter_created_after.set(if ca.is_empty() { None } else { Some(ca) });
        set_filter_created_before.set(if cb.is_empty() { None } else { Some(cb) });
        set_filter_status.set(if s.is_empty() { None } else { Some(s) });
    };

    let clear_filters = move |_| {
        set_local_owner.set(String::new());
        set_local_name.set(String::new());
        set_local_repo_url.set(String::new());
        set_local_workflow_path.set(String::new());
        set_local_created_after.set(String::new());
        set_local_created_before.set(String::new());
        set_local_status.set("Any".to_string());

        set_filter_owner.set(None);
        set_filter_name.set(None);
        set_filter_repo_url.set(None);
        set_filter_workflow_path.set(None);
        set_filter_created_after.set(None);
        set_filter_created_before.set(None);
        set_filter_status.set(Some("Any".to_string()));
    };

    let statuses = vec![
        "Any",
        "queued",
        "resolving",
        "start_pending",
        "running",
        "succeeded",
        "failed",
    ];

    view! {
        <div style="border-bottom: 1px solid var(--surface-border); padding: 1rem; flex: none;">
            <div style="display: flex; justify-content: space-between; align-items: center; cursor: pointer; user-select: none;" on:click=move |_| set_is_expanded.update(|e| *e = !*e)>
                <h3 style="margin: 0; display: flex; align-items: center; gap: 0.5rem; font-size: 1rem;">
                    "Filters"
                </h3>
                <span>{move || if is_expanded.get() { "▲" } else { "▼" }}</span>
            </div>

            <div style=move || format!("display: {}; margin-top: 1rem;", if is_expanded.get() { "block" } else { "none" })>
                <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 1rem; margin-bottom: 1rem;">
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Owner"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_owner on:input=move |ev| set_local_owner.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Name"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_name on:input=move |ev| set_local_name.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Repo URL"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_repo_url on:input=move |ev| set_local_repo_url.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Workflow Path"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_workflow_path on:input=move |ev| set_local_workflow_path.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Status"</label>
                        <select style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" on:change=move |ev| set_local_status.set(event_target_value(&ev))>
                            {statuses.into_iter().map(|s| {
                                let s_clone = s.to_string();
                                view! {
                                    <option value=s selected=move || local_status.get() == s_clone style="background: var(--surface-elevated); color: var(--text-primary);">
                                        {s}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Created After"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" placeholder="e.g. 2024-01-01T00:00:00Z" prop:value=local_created_after on:input=move |ev| set_local_created_after.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Created Before"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" placeholder="e.g. 2024-01-01T00:00:00Z" prop:value=local_created_before on:input=move |ev| set_local_created_before.set(event_target_value(&ev)) />
                    </div>
                </div>

                <div style="display: flex; justify-content: flex-end; gap: 0.5rem;">
                    <button style="padding: 0.5rem 1rem; border-radius: 4px; border: 1px solid var(--surface-border); background: transparent; color: var(--text-primary); cursor: pointer;" on:click=clear_filters>"Clear"</button>
                    <button style="padding: 0.5rem 1rem; border-radius: 4px; border: none; background: var(--primary-color); color: white; cursor: pointer; font-weight: 500;" on:click=apply_filters>"Apply Filters"</button>
                </div>
            </div>
        </div>
    }
}
