use crate::api::{create_storage_backend, fetch_connections};
use crate::models::Connection;
use leptos::prelude::*;
use leptos::task::spawn_local;
use stormchaser_model::connections::ConnectionType;

#[component]
pub fn ConnectionsTable(connections: Vec<Connection>) -> impl IntoView {
    if connections.is_empty() {
        return view! {
            <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">
                "No backend connections found."
            </div>
        }
        .into_any();
    }

    view! {
        <table class="glass-panel" style="width: 100%; border-collapse: separate; border-spacing: 0;">
            <thead>
                <tr>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Name"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Type"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Description"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: left; font-weight: 600; color: var(--text-secondary);">"Default SFS"</th>
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; font-weight: 600; color: var(--text-secondary);">"Last Updated"</th>
                </tr>
            </thead>
            <tbody>
                {connections.into_iter().map(|conn| {
                    let default_sfs_badge = if conn.is_default_sfs {
                        view! { <span class="status-badge status-succeeded">"Yes"</span> }.into_any()
                    } else {
                        view! { <span class="status-badge status-queued">"No"</span> }.into_any()
                    };

                    view! {
                        <tr style="transition: background-color 0.2s;">
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <strong>{conn.name}</strong>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                <span class="status-badge status-running">{conn.connection_type.to_string()}</span>
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); color: var(--text-secondary);">
                                {conn.description.unwrap_or_else(|| "-".to_string())}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                                {default_sfs_badge}
                            </td>
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; color: var(--text-secondary); font-variant-numeric: tabular-nums;">
                                {conn.updated_at.format("%Y-%m-%d %H:%M:%S").to_string()}
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
pub fn CreateBackendModal(on_close: Callback<()>, on_success: Callback<()>) -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (description, set_description) = signal(String::new());
    let (connection_type, set_connection_type) = signal(ConnectionType::Postgres);
    let (config_json, set_config_json) = signal(String::new());
    let (aws_assume_role_arn, set_aws_assume_role_arn) = signal(String::new());
    let (is_default_sfs, set_is_default_sfs) = signal(false);

    let (is_submitting, set_is_submitting) = signal(false);
    let (error_message, set_error_message) = signal(Option::<String>::None);

    let submit = move |_| {
        if name.get().is_empty() || config_json.get().is_empty() {
            set_error_message.set(Some("Name and Config JSON are required".to_string()));
            return;
        }

        let config_val: serde_json::Value = match serde_json::from_str(&config_json.get()) {
            Ok(v) => v,
            Err(_) => {
                set_error_message.set(Some("Invalid JSON in Config".to_string()));
                return;
            }
        };

        set_is_submitting.set(true);
        set_error_message.set(None);

        let n = name.get();
        let d = description.get();
        let desc = if d.is_empty() { None } else { Some(d) };
        let c_type = connection_type.get();
        let arn = aws_assume_role_arn.get();
        let a_arn = if arn.is_empty() { None } else { Some(arn) };
        let default_sfs = is_default_sfs.get();

        spawn_local(async move {
            match create_storage_backend(n, desc, c_type, config_val, a_arn, default_sfs).await {
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
                    <h2>"Create Backend Connection"</h2>
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
                        <label class="form-label">"Connection Type *"</label>
                        <select
                            class="input-field"
                            on:change=move |ev| {
                                let val = event_target_value(&ev);
                                let ct = serde_json::from_str::<ConnectionType>(&format!("\"{}\"", val)).unwrap_or(ConnectionType::Postgres);
                                set_connection_type.set(ct);
                            }
                        >
                                                        <option value="Postgres" selected=move || connection_type.get() == ConnectionType::Postgres>"Postgres"</option>
                            <option value="Mysql" selected=move || connection_type.get() == ConnectionType::Mysql>"MySQL"</option>
                            <option value="HttpApi" selected=move || connection_type.get() == ConnectionType::HttpApi>"HTTP API"</option>
                            <option value="Git" selected=move || connection_type.get() == ConnectionType::Git>"Git"</option>
                            <option value="S3" selected=move || connection_type.get() == ConnectionType::S3>"S3 / MinIO"</option>
                            <option value="Gcs" selected=move || connection_type.get() == ConnectionType::Gcs>"GCS"</option>
                            <option value="Azure" selected=move || connection_type.get() == ConnectionType::Azure>"Azure Blob"</option>
                            <option value="Oci" selected=move || connection_type.get() == ConnectionType::Oci>"OCI Registry"</option>
                            <option value="Jfrog" selected=move || connection_type.get() == ConnectionType::Jfrog>"JFrog"</option>
                        </select>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Config JSON *"</label>
                        <textarea
                            class="input-field"
                            style="min-height: 120px; font-family: monospace;"
                            placeholder="{\n  \"url\": \"postgres://...\"\n}"
                            prop:value=config_json
                            on:input=move |ev| set_config_json.set(event_target_value(&ev))
                        />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"AWS Assume Role ARN (Optional)"</label>
                        <input type="text" class="input-field" placeholder="arn:aws:iam::..." prop:value=aws_assume_role_arn on:input=move |ev| set_aws_assume_role_arn.set(event_target_value(&ev)) />
                    </div>

                    <div class="form-group" style="display: flex; align-items: center; gap: 0.5rem;">
                        <input type="checkbox" id="is_default_sfs" prop:checked=is_default_sfs on:change=move |ev| set_is_default_sfs.set(event_target_checked(&ev)) />
                        <label for="is_default_sfs" class="form-label" style="margin: 0;">"Use as default SFS backend"</label>
                    </div>
                </div>

                <div class="modal-footer" style="display: flex; justify-content: flex-end; gap: 1rem; padding: 1.5rem; border-top: 1px solid var(--surface-border);">
                    <button class="btn-secondary" on:click=move |_| on_close.run(()) disabled=move || is_submitting.get()>
                        "Cancel"
                    </button>
                    <button class="btn-primary" on:click=submit disabled=move || is_submitting.get()>
                        {move || if is_submitting.get() { "Creating..." } else { "Create Connection" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

#[component]
pub fn BackendsList() -> impl IntoView {
    let connections_resource = Resource::new(|| (), |_| async { fetch_connections().await });
    let (show_create_modal, set_show_create_modal) = signal(false);

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            {move || if show_create_modal.get() {
                view! {
                    <CreateBackendModal
                        on_close=Callback::new(move |_| set_show_create_modal.set(false))
                        on_success=Callback::new(move |_| {
                            set_show_create_modal.set(false);
                            connections_resource.refetch();
                        })
                    />
                }.into_any()
            } else { ().into_any() }}
            <div class="glass-panel table-container" style="flex: 1; overflow: auto;">
                <div style="padding: 1.5rem; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem; font-weight: 600;">"Backend Connections"</h2>
                    <button class="btn-primary" on:click=move |_| set_show_create_modal.set(true)>
                        "Create Connection"
                    </button>
                </div>

                <Suspense fallback=|| view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading connections..."</div> }>
                    {move || match connections_resource.get() {
                        Some(Ok(connections)) => view! {
                            <ConnectionsTable connections=connections />
                        }.into_any(),
                        Some(Err(e)) => view! {
                            <div style="padding: 2rem; text-align: center; color: var(--status-error);">
                                "Error loading connections: " {e.to_string()}
                            </div>
                        }.into_any(),
                        None => view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading connections..."</div> }.into_any(),
                    }}
                </Suspense>
            </div>
        </div>
    }
}
