use crate::api::{create_event_rule, fetch_event_rules};
use crate::models::EventRule;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn RulesTable(rules: Vec<EventRule>, on_delete_success: Callback<()>) -> impl IntoView {
    if rules.is_empty() {
        return view! {
            <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">
                "No event rules configured."
            </div>
        }
        .into_any();
    }

    view! {
        <table class="glass-panel" style="width: 100%; border-collapse: separate; border-spacing: 0;">
            <thead>
                <tr>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Name"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Workflow"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Event Pattern"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Status"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; font-weight: 600; color: var(--text-secondary);">"Last Updated"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; font-weight: 600; color: var(--text-secondary);">"Actions"</th>
                </tr>
            </thead>
            <tbody>
                {rules.into_iter().map(|rule| {
                    let status_badge = if rule.is_active {
                        view! { <span class="status-badge status-succeeded">"Active"</span> }.into_any()
                    } else {
                        view! { <span class="status-badge status-queued">"Inactive"</span> }.into_any()
                    };

                    view! {
                        <tr style="transition: background-color 0.2s;">
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <strong>{rule.name}</strong>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <span class="status-badge status-running">{rule.workflow_name}</span>
                                <div style="font-size: 0.75rem; color: var(--text-secondary); margin-top: 0.25rem;">
                                    {rule.repo_url}
                                </div>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); color: var(--text-secondary);">
                                <code>{rule.event_type_pattern}</code>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                {status_badge}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; color: var(--text-secondary); font-variant-numeric: tabular-nums;">
                                {rule.updated_at.format("%Y-%m-%d %H:%M:%S").to_string()}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right;">
                                <button
                                    class="icon-btn"
                                    style="color: var(--status-error); padding: 0.25rem;"
                                    title="Delete"
                                    on:click={
                                        let id = rule.id.to_string();
                                        let on_success = on_delete_success;
                                        move |_| {
                                            let id = id.clone();
                                            spawn_local(async move {
                                                if crate::api::delete_event_rule(id).await.is_ok() {
                                                    on_success.run(());
                                                }
                                            });
                                        }
                                    }
                                >
                                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                        <path d="M3 6h18"></path>
                                        <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"></path>
                                        <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"></path>
                                    </svg>
                                </button>
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
pub fn CreateRuleModal(on_close: Callback<()>, on_success: Callback<()>) -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (description, set_description) = signal(String::new());
    let (webhook_id, set_webhook_id) = signal(String::new());
    let (event_pattern, set_event_pattern) = signal(String::new());
    let (condition_expr, set_condition_expr) = signal(String::new());

    let (workflow_name, set_workflow_name) = signal(String::new());
    let (selected_connection, set_selected_connection) = signal(String::new());
    let (workflow_path, set_workflow_path) = signal(String::new());
    let (git_ref, set_git_ref) = signal(String::new());

    let (input_mappings, set_input_mappings) = signal(String::new());

    let (is_submitting, set_is_submitting) = signal(false);
    let (error_message, set_error_message) = signal(Option::<String>::None);

    let connections_res = Resource::new(
        || (),
        |_| async { crate::api::fetch_connections().await.unwrap_or_default() },
    );

    let submit = move |_| {
        if name.get().is_empty()
            || webhook_id.get().is_empty()
            || event_pattern.get().is_empty()
            || workflow_name.get().is_empty()
            || selected_connection.get().is_empty()
            || workflow_path.get().is_empty()
            || git_ref.get().is_empty()
        {
            set_error_message.set(Some("All required fields must be filled".to_string()));
            return;
        }

        let input_map_val: std::collections::HashMap<String, String> =
            if input_mappings.get().trim().is_empty() {
                std::collections::HashMap::new()
            } else {
                match serde_json::from_str(&input_mappings.get()) {
                    Ok(v) => v,
                    Err(_) => {
                        set_error_message.set(Some("Invalid JSON in Input Mappings".to_string()));
                        return;
                    }
                }
            };

        set_is_submitting.set(true);
        set_error_message.set(None);

        let n = name.get();
        let d = description.get();
        let desc = if d.is_empty() { None } else { Some(d) };
        let w_id = webhook_id.get();
        let ev_pat = event_pattern.get();
        let cond_expr = condition_expr.get();
        let cond = if cond_expr.is_empty() {
            None
        } else {
            Some(cond_expr)
        };
        let w_n = workflow_name.get();
        let conn = selected_connection.get();
        let w_p = workflow_path.get();
        let g_r = git_ref.get();

        spawn_local(async move {
            match create_event_rule(
                n,
                desc,
                w_id,
                ev_pat,
                cond,
                w_n,
                conn,
                w_p,
                g_r,
                input_map_val,
            )
            .await
            {
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
                    <h2>"Create Event Rule"</h2>
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

                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem;">
                        <div class="form-group">
                            <label class="form-label">"Webhook ID *"</label>
                            <input type="text" class="input-field" prop:value=webhook_id on:input=move |ev| set_webhook_id.set(event_target_value(&ev)) />
                        </div>
                        <div class="form-group">
                            <label class="form-label">"Event Pattern *"</label>
                            <input type="text" class="input-field" placeholder="e.g. push" prop:value=event_pattern on:input=move |ev| set_event_pattern.set(event_target_value(&ev)) />
                        </div>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Condition Expr"</label>
                        <input type="text" class="input-field" placeholder="Optional HCL condition" prop:value=condition_expr on:input=move |ev| set_condition_expr.set(event_target_value(&ev)) />
                    </div>

                    <div style="border-top: 1px solid var(--surface-border); margin: 1rem 0;"></div>
                    <h3 style="margin: 0; font-size: 1rem; color: var(--text-secondary);">"Target Workflow"</h3>

                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem;">
                        <div class="form-group">
                            <label class="form-label">"Workflow Name *"</label>
                            <input type="text" class="input-field" prop:value=workflow_name on:input=move |ev| set_workflow_name.set(event_target_value(&ev)) />
                        </div>
                        <div class="form-group">
                            <label class="form-label">"Git Connection *"</label>
                            <Suspense fallback=|| view! { <div>"Loading connections..."</div> }>
                                {move || {
                                    let conns = connections_res.get().unwrap_or_default();
                                    let git_conns: Vec<_> = conns.into_iter().filter(|c| matches!(c.connection_type, crate::models::ConnectionType::Git)).collect();

                                    view! {
                                        <select
                                            class="input-field"
                                            on:change=move |ev| set_selected_connection.set(event_target_value(&ev))
                                        >
                                            <option value="" disabled=true selected=move || selected_connection.get().is_empty()>"Select a connection"</option>
                                            {git_conns.into_iter().map(|c| {
                                                let id_str = c.id.to_string();
                                                view! {
                                                    <option value={id_str.clone()} selected=move || selected_connection.get() == id_str>
                                                        {c.name}
                                                    </option>
                                                }
                                            }).collect_view()}
                                        </select>
                                    }
                                }}
                            </Suspense>
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
                        <label class="form-label">"Input Mappings (JSON)"</label>
                        <textarea
                            class="input-field"
                            style="min-height: 80px; font-family: monospace;"
                            placeholder="{\n  \"my_input\": \"${event.payload.ref}\"\n}"
                            prop:value=input_mappings
                            on:input=move |ev| set_input_mappings.set(event_target_value(&ev))
                        />
                    </div>
                </div>

                <div class="modal-footer" style="display: flex; justify-content: flex-end; gap: 1rem; padding: 1.5rem; border-top: 1px solid var(--surface-border);">
                    <button class="btn-secondary" on:click=move |_| on_close.run(()) disabled=move || is_submitting.get()>
                        "Cancel"
                    </button>
                    <button class="btn-primary" on:click=submit disabled=move || is_submitting.get()>
                        {move || if is_submitting.get() { "Creating..." } else { "Create Rule" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
pub fn RulesList() -> impl IntoView {
    let rules_resource = Resource::new(|| (), |_| async { fetch_event_rules().await });
    let (show_create_modal, set_show_create_modal) = signal(false);

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            {move || if show_create_modal.get() {
                view! {
                    <CreateRuleModal
                        on_close=Callback::new(move |_| set_show_create_modal.set(false))
                        on_success=Callback::new(move |_| {
                            set_show_create_modal.set(false);
                            rules_resource.refetch();
                        })
                    />
                }.into_any()
            } else { ().into_any() }}
            <div class="glass-panel table-container" style="flex: 1; overflow: auto;">
                <div style="padding: 1.5rem; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem; font-weight: 600;">"Event Rules"</h2>
                    <button class="btn-primary" on:click=move |_| set_show_create_modal.set(true)>
                        "Create Rule"
                    </button>
                </div>

                <Suspense fallback=|| view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading event rules..."</div> }>
                    {move || match rules_resource.get() {
                        Some(Ok(rules)) => view! {
                            <RulesTable rules=rules on_delete_success=Callback::new(move |_| rules_resource.refetch()) />
                        }.into_any(),
                        Some(Err(e)) => view! {
                            <div style="padding: 2rem; text-align: center; color: var(--status-error);">
                                "Error loading rules: " {e.to_string()}
                            </div>
                        }.into_any(),
                        None => view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading event rules..."</div> }.into_any(),
                    }}
                </Suspense>
            </div>
        </div>
    }
}
