use crate::api::{create_webhook, fetch_webhooks};
use crate::models::WebhookConfig;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn WebhooksTable(
    webhooks: Vec<WebhookConfig>,
    on_delete_success: Callback<()>,
) -> impl IntoView {
    if webhooks.is_empty() {
        return view! {
            <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">
                "No webhooks configured."
            </div>
        }
        .into_any();
    }

    view! {
        <table class="glass-panel" style="width: 100%; border-collapse: separate; border-spacing: 0;">
            <thead>
                <tr>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Name"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Source Type"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Description"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Status"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; font-weight: 600; color: var(--text-secondary);">"Last Updated"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; font-weight: 600; color: var(--text-secondary);">"Actions"</th>
                </tr>
            </thead>
            <tbody>
                {webhooks.into_iter().map(|wh| {
                    let status_badge = if wh.is_active {
                        view! { <span class="status-badge status-succeeded">"Active"</span> }.into_any()
                    } else {
                        view! { <span class="status-badge status-queued">"Inactive"</span> }.into_any()
                    };

                    view! {
                        <tr style="transition: background-color 0.2s;">
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <strong>{wh.name}</strong>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <span class="status-badge status-running">{wh.source_type}</span>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); color: var(--text-secondary);">
                                {wh.description.unwrap_or_else(|| "-".to_string())}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                {status_badge}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; color: var(--text-secondary); font-variant-numeric: tabular-nums;">
                                {wh.updated_at.format("%Y-%m-%d %H:%M:%S").to_string()}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right;">
                                <button
                                    class="icon-btn"
                                    style="color: var(--status-error); padding: 0.25rem;"
                                    title="Delete"
                                    on:click={
                                        let id = wh.id.to_string();
                                        let on_success = on_delete_success;
                                        move |_| {
                                            let id = id.clone();
                                            spawn_local(async move {
                                                if crate::api::delete_webhook(id).await.is_ok() {
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
pub fn CreateWebhookModal(on_close: Callback<()>, on_success: Callback<()>) -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (description, set_description) = signal(String::new());
    let (source_type, set_source_type) = signal("github".to_string());
    let (secret_token, set_secret_token) = signal(String::new());

    let (is_submitting, set_is_submitting) = signal(false);
    let (error_message, set_error_message) = signal(Option::<String>::None);

    let submit = move |_| {
        if name.get().is_empty() {
            set_error_message.set(Some("Name is required".to_string()));
            return;
        }

        set_is_submitting.set(true);
        set_error_message.set(None);

        let n = name.get();
        let d = description.get();
        let desc = if d.is_empty() { None } else { Some(d) };
        let src = source_type.get();
        let s = secret_token.get();
        let sec = if s.is_empty() { None } else { Some(s) };

        spawn_local(async move {
            match create_webhook(n, desc, src, sec).await {
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
            <div class="glass-panel modal-content" on:click=|e| e.stop_propagation() style="max-width: 500px; width: 100%;">
                <div class="modal-header">
                    <h2>"Create Webhook"</h2>
                    <button class="icon-btn" on:click=move |_| on_close.run(()) title="Close">
                        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <line x1="18" y1="6" x2="6" y2="18"></line>
                            <line x1="6" y1="6" x2="18" y2="18"></line>
                        </svg>
                    </button>
                </div>

                <div class="modal-body" style="display: flex; flex-direction: column; gap: 1.5rem; padding: 1.5rem;">
                    {move || error_message.get().map(|msg| view! {
                        <div style="padding: 1rem; background: rgba(239, 68, 68, 0.1); border: 1px solid rgba(239, 68, 68, 0.2); border-radius: 6px; color: var(--status-error);">
                            {msg}
                        </div>
                    })}

                    <div class="form-group">
                        <label class="form-label">"Name *"</label>
                        <input
                            type="text"
                            class="input-field"
                            placeholder="e.g. github-main"
                            prop:value=name
                            on:input=move |ev| set_name.set(event_target_value(&ev))
                        />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Description"</label>
                        <input
                            type="text"
                            class="input-field"
                            placeholder="Optional"
                            prop:value=description
                            on:input=move |ev| set_description.set(event_target_value(&ev))
                        />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Source Type"</label>
                        <select
                            class="input-field"
                            on:change=move |ev| set_source_type.set(event_target_value(&ev))
                        >
                            <option value="github" selected=move || source_type.get() == "github">"GitHub"</option>
                            <option value="generic" selected=move || source_type.get() == "generic">"Generic"</option>
                        </select>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Secret Token"</label>
                        <input
                            type="password"
                            class="input-field"
                            placeholder="Optional webhook secret"
                            prop:value=secret_token
                            on:input=move |ev| set_secret_token.set(event_target_value(&ev))
                        />
                    </div>
                </div>

                <div class="modal-footer" style="display: flex; justify-content: flex-end; gap: 1rem; padding: 1.5rem; border-top: 1px solid var(--surface-border);">
                    <button
                        class="btn-secondary"
                        on:click=move |_| on_close.run(())
                        disabled=move || is_submitting.get()
                    >
                        "Cancel"
                    </button>
                    <button
                        class="btn-primary"
                        on:click=submit
                        disabled=move || is_submitting.get() || name.get().is_empty()
                    >
                        {move || if is_submitting.get() { "Creating..." } else { "Create Webhook" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
pub fn WebhooksList() -> impl IntoView {
    let webhooks_resource = Resource::new(|| (), |_| async { fetch_webhooks().await });
    let (show_create_modal, set_show_create_modal) = signal(false);

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            {move || if show_create_modal.get() {
                view! {
                    <CreateWebhookModal
                        on_close=Callback::new(move |_| set_show_create_modal.set(false))
                        on_success=Callback::new(move |_| {
                            set_show_create_modal.set(false);
                            webhooks_resource.refetch();
                        })
                    />
                }.into_any()
            } else { ().into_any() }}
            <div class="glass-panel table-container" style="flex: 1; overflow: auto;">
                <div style="padding: 1.5rem; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem; font-weight: 600;">"Webhooks"</h2>
                    <button class="btn-primary" on:click=move |_| set_show_create_modal.set(true)>
                        "Create Webhook"
                    </button>
                </div>

                <Suspense fallback=|| view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading webhooks..."</div> }>
                    {move || match webhooks_resource.get() {
                        Some(Ok(webhooks)) => view! {
                            <WebhooksTable webhooks=webhooks on_delete_success=Callback::new(move |_| webhooks_resource.refetch()) />
                        }.into_any(),
                        Some(Err(e)) => view! {
                            <div style="padding: 2rem; text-align: center; color: var(--status-error);">
                                "Error loading webhooks: " {e.to_string()}
                            </div>
                        }.into_any(),
                        None => view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading webhooks..."</div> }.into_any(),
                    }}
                </Suspense>
            </div>
        </div>
    }
}
