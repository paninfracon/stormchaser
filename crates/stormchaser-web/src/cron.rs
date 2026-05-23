use crate::api::{create_cron_workflow, fetch_cron_workflows};
use crate::models::CronWorkflow;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn CronTable(crons: Vec<CronWorkflow>) -> impl IntoView {
    if crons.is_empty() {
        return view! {
            <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">
                "No cron workflows configured."
            </div>
        }
        .into_any();
    }

    view! {
        <table class="glass-panel" style="width: 100%; border-collapse: separate; border-spacing: 0;">
            <thead>
                <tr>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Name"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Schedule"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Workflow"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Status"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; font-weight: 600; color: var(--text-secondary);">"Last Updated"</th>
                </tr>
            </thead>
            <tbody>
                {crons.into_iter().map(|cron| {
                    let status_badge = if cron.is_active {
                        view! { <span class="status-badge status-succeeded">"Active"</span> }.into_any()
                    } else {
                        view! { <span class="status-badge status-queued">"Inactive"</span> }.into_any()
                    };

                    view! {
                        <tr style="transition: background-color 0.2s;">
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <strong>{cron.name}</strong>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <span class="status-badge status-running" style="font-family: monospace;">{cron.cronspec}</span>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); color: var(--text-secondary);">
                                {cron.workflow_name}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                {status_badge}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; color: var(--text-secondary); font-variant-numeric: tabular-nums;">
                                {cron.updated_at.format("%Y-%m-%d %H:%M:%S").to_string()}
                            </td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

#[component]
pub fn CreateCronModal(on_close: Callback<()>, on_success: Callback<()>) -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (description, set_description) = signal(String::new());
    let (cronspec, set_cronspec) = signal(String::new());

    let (workflow_name, set_workflow_name) = signal(String::new());
    let (repo_url, set_repo_url) = signal(String::new());
    let (workflow_path, set_workflow_path) = signal(String::new());
    let (git_ref, set_git_ref) = signal(String::new());

    let (inputs_json, set_inputs_json) = signal(String::new());

    let (is_submitting, set_is_submitting) = signal(false);
    let (error_message, set_error_message) = signal(Option::<String>::None);

    let submit = move |_| {
        if name.get().is_empty()
            || cronspec.get().is_empty()
            || workflow_name.get().is_empty()
            || repo_url.get().is_empty()
            || workflow_path.get().is_empty()
            || git_ref.get().is_empty()
        {
            set_error_message.set(Some("All required fields must be filled".to_string()));
            return;
        }

        let inputs_val: serde_json::Value = if inputs_json.get().trim().is_empty() {
            serde_json::json!({})
        } else {
            match serde_json::from_str(&inputs_json.get()) {
                Ok(v) => v,
                Err(_) => {
                    set_error_message.set(Some("Invalid JSON in Inputs".to_string()));
                    return;
                }
            }
        };

        set_is_submitting.set(true);
        set_error_message.set(None);

        let n = name.get();
        let d = description.get();
        let desc = if d.is_empty() { None } else { Some(d) };
        let c = cronspec.get();
        let w_n = workflow_name.get();
        let r_u = repo_url.get();
        let w_p = workflow_path.get();
        let g_r = git_ref.get();

        spawn_local(async move {
            match create_cron_workflow(n, desc, c, w_n, r_u, w_p, g_r, inputs_val).await {
                Ok(_) => {
                    set_is_submitting.set(false);
                    on_success.run(());
                }
                Err(e) => {
                    set_is_submitting.set(false);
                    set_error_message.set(Some(e.to_string()));
                }
            }
        });
    };

    view! {
        <div class="modal-overlay" on:click=move |_| on_close.run(())>
            <div class="glass-panel modal-content" on:click=|e| e.stop_propagation() style="max-width: 600px; width: 100%; max-height: 90vh; display: flex; flex-direction: column;">
                <div class="modal-header">
                    <h2>"Create Cron Workflow"</h2>
                    <button class="icon-btn" on:click=move |_| on_close.run(()) title="Close">
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <line x1="18" y1="6" x2="6" y2="18"></line>
                            <line x1="6" y1="6" x2="18" y2="18"></line>
                        </svg>
                    </button>
                </div>

                <div class="modal-body" style="display: flex; flex-direction: column; gap: 1.5rem; padding: 1.5rem; overflow-y: auto;">
                    {move || error_message.get().map(|msg| view! {
                        <div style="padding: 1rem; background: rgba(239, 68, 68, 0.1); border: 1px solid rgba(239, 68, 68, 0.2); border-radius: 6px; color: var(--status-error);">
                            {msg}
                        </div>
                    })}

                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem;">
                        <div class="form-group">
                            <label class="form-label">"Name *"</label>
                            <input type="text" class="input-field" prop:value=name on:input=move |ev| set_name.set(event_target_value(&ev)) />
                        </div>
                        <div class="form-group">
                            <label class="form-label">"Description"</label>
                            <input type="text" class="input-field" prop:value=description on:input=move |ev| set_description.set(event_target_value(&ev)) />
                        </div>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Cron Expression *"</label>
                        <input type="text" class="input-field" placeholder="e.g. 0 0 * * *" prop:value=cronspec on:input=move |ev| set_cronspec.set(event_target_value(&ev)) />
                    </div>

                    <div style="border-top: 1px solid var(--surface-border); margin: 1rem 0;"></div>
                    <h3 style="margin: 0; font-size: 1rem; color: var(--text-secondary);">"Target Workflow"</h3>

                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem;">
                        <div class="form-group">
                            <label class="form-label">"Workflow Name *"</label>
                            <input type="text" class="input-field" prop:value=workflow_name on:input=move |ev| set_workflow_name.set(event_target_value(&ev)) />
                        </div>
                        <div class="form-group">
                            <label class="form-label">"Repository URL *"</label>
                            <input type="text" class="input-field" prop:value=repo_url on:input=move |ev| set_repo_url.set(event_target_value(&ev)) />
                        </div>
                    </div>

                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem;">
                        <div class="form-group">
                            <label class="form-label">"Workflow Path *"</label>
                            <input type="text" class="input-field" prop:value=workflow_path on:input=move |ev| set_workflow_path.set(event_target_value(&ev)) />
                        </div>
                        <div class="form-group">
                            <label class="form-label">"Git Ref *"</label>
                            <input type="text" class="input-field" prop:value=git_ref on:input=move |ev| set_git_ref.set(event_target_value(&ev)) />
                        </div>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Workflow Inputs (JSON)"</label>
                        <textarea
                            class="input-field"
                            style="min-height: 80px; font-family: monospace;"
                            placeholder="{\n  \"env\": \"prod\"\n}"
                            prop:value=inputs_json
                            on:input=move |ev| set_inputs_json.set(event_target_value(&ev))
                        />
                    </div>
                </div>

                <div class="modal-footer" style="display: flex; justify-content: flex-end; gap: 1rem; padding: 1.5rem; border-top: 1px solid var(--surface-border);">
                    <button class="btn-secondary" on:click=move |_| on_close.run(()) disabled=move || is_submitting.get()>
                        "Cancel"
                    </button>
                    <button class="btn-primary" on:click=submit disabled=move || is_submitting.get()>
                        {move || if is_submitting.get() { "Creating..." } else { "Create Cron" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
pub fn CronList() -> impl IntoView {
    let crons_resource = Resource::new(|| (), |_| async { fetch_cron_workflows().await });
    let (show_create_modal, set_show_create_modal) = signal(false);

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            {move || if show_create_modal.get() {
                view! {
                    <CreateCronModal
                        on_close=Callback::new(move |_| set_show_create_modal.set(false))
                        on_success=Callback::new(move |_| {
                            set_show_create_modal.set(false);
                            crons_resource.refetch();
                        })
                    />
                }.into_any()
            } else { ().into_any() }}
            <div class="glass-panel table-container" style="flex: 1; overflow: auto;">
                <div style="padding: 1.5rem; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem; font-weight: 600;">"Cron Workflows"</h2>
                    <button class="btn-primary" on:click=move |_| set_show_create_modal.set(true)>
                        "Create Cron"
                    </button>
                </div>

                <Suspense fallback=|| view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading cron workflows..."</div> }>
                    {move || match crons_resource.get() {
                        Some(Ok(crons)) => view! {
                            <CronTable crons=crons />
                        }.into_any(),
                        Some(Err(e)) => view! {
                            <div style="padding: 2rem; text-align: center; color: var(--status-error);">
                                "Error loading crons: " {e.to_string()}
                            </div>
                        }.into_any(),
                        None => view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading cron workflows..."</div> }.into_any(),
                    }}
                </Suspense>
            </div>
        </div>
    }
}
