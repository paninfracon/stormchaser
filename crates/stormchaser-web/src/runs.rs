use crate::api::fetch_workflow_runs;
use crate::models::{RunStatus, WorkflowRunDetail};
use leptos::prelude::*;

#[component]
pub fn WorkflowRunsList() -> impl IntoView {
    // We use a Resource to fetch the workflow runs from the server function.
    let runs_resource = Resource::new(|| (), |_| async { fetch_workflow_runs(None).await });

    // Try to get initial theme from local storage or default to dark
    let initial_is_dark = {
        #[cfg(not(feature = "ssr"))]
        {
            let window = leptos::prelude::window();
            if let Ok(Some(storage)) = window.local_storage() {
                if let Ok(Some(val)) = storage.get_item("theme") {
                    val == "dark"
                } else {
                    true
                }
            } else {
                true
            }
        }
        #[cfg(feature = "ssr")]
        {
            true
        }
    };

    let (is_dark, set_is_dark) = signal(initial_is_dark);

    let _theme_effect = Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            let window = leptos::prelude::window();
            let dark = is_dark.get();
            if let Some(document) = window.document() {
                if let Some(body) = document.body() {
                    let _ = body.set_attribute("data-theme", if dark { "dark" } else { "light" });
                }
            }
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item("theme", if dark { "dark" } else { "light" });
            }
        }
    });

    #[cfg(not(feature = "ssr"))]
    on_cleanup(move || {
        std::mem::drop(_theme_effect);
    });

    let toggle_theme = move |_| {
        set_is_dark.update(|d| *d = !*d);
    };

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
            std::mem::drop(_sse_effect);
        });
    }

    view! {
        <div class="page-container fade-in">
            <div class="header">
                <h1>"Workflow Runs"</h1>
                <div style="display: flex; gap: 1rem; align-items: center;">
                    <button class="btn" style="background: transparent; color: var(--text-primary); border: 1px solid var(--surface-border);" on:click=toggle_theme>
                        {move || if is_dark.get() { "☀️ Light Mode" } else { "🌙 Dark Mode" }}
                    </button>
                    <Suspense fallback=|| view! { <a href="/auth/login" class="btn" rel="external">"Login with Dex"</a> }>
                        {move || match runs_resource.get() {
                            Some(Ok(_)) => view! {
                                <a href="/auth/logout" class="btn" rel="external">
                                    "Logout"
                                </a>
                            }.into_any(),
                            _ => view! {
                                <a href="/auth/login" class="btn" rel="external">"Login with Dex"</a>
                            }.into_any()
                        }}
                    </Suspense>
                </div>
            </div>

            <div class="glass-panel table-container">
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
                                view! { <RunsTable runs=runs_list /> }.into_any()
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
    }
}

#[component]
fn RunsTable(#[prop(into)] runs: Signal<Vec<WorkflowRunDetail>>) -> impl IntoView {
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
                        <tr>
                            <td style="font-weight: 500;">{run.workflow_name}</td>
                            <td style="color: var(--text-secondary);">{run.initiating_user}</td>
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
