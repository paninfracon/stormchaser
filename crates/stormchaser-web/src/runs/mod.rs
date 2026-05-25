pub mod details;
pub mod filters;
pub mod logs;
pub mod table;

use details::*;
use filters::*;
use logs::*;
use table::*;

use crate::api::fetch_workflow_runs;
use crate::run_modal::CreateRunModal;
use leptos::prelude::*;

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
                            <RunDetailsPanel run_id=run_id selected_step_id=selected_step_id set_selected_step_id=set_selected_step_id set_selected_run_id=set_selected_run_id />
                        </div>
                    }
                })}
            </div>
    }
}
