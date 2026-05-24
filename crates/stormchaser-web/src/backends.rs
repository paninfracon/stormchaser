use crate::api::{create_storage_backend, fetch_connections};
use crate::models::Connection;
use leptos::prelude::*;
use leptos::task::spawn_local;
use stormchaser_model::connections::ConnectionType;

#[component]
pub fn ConnectionsTable(
    connections: Vec<Connection>,
    on_delete_success: Callback<()>,
) -> impl IntoView {
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
                    <th style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right; font-weight: 600; color: var(--text-secondary);">"Actions"</th>
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
                            <td style="padding: 1rem; border-bottom: 1px solid var(--surface-border); text-align: right;">
                                <button
                                    class="icon-btn"
                                    style="color: var(--status-error); padding: 0.25rem;"
                                    title="Delete"
                                    on:click={
                                        let id = conn.id.to_string();
                                        let on_success = on_delete_success;
                                        move |_| {
                                            let id = id.clone();
                                            spawn_local(async move {
                                                if crate::api::delete_connection(id).await.is_ok() {
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
pub fn CreateBackendModal(on_close: Callback<()>, on_success: Callback<()>) -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (description, set_description) = signal(String::new());
    let (connection_type, set_connection_type) = signal(ConnectionType::Postgres);
    let (url, set_url) = signal(String::new());
    let (bucket, set_bucket) = signal(String::new());
    let (region, set_region) = signal(String::new());
    let (endpoint, set_endpoint) = signal(String::new());
    let (access_key, set_access_key) = signal(String::new());
    let (secret_key, set_secret_key) = signal(String::new());
    let (force_path_style, set_force_path_style) = signal(false);
    let (username, set_username) = signal(String::new());
    let (ssh_key, set_ssh_key) = signal(String::new());
    let (aws_assume_role_arn, set_aws_assume_role_arn) = signal(String::new());
    let (is_default_sfs, set_is_default_sfs) = signal(false);

    let (is_submitting, set_is_submitting) = signal(false);
    let (is_testing, set_is_testing) = signal(false);
    let (test_result, set_test_result) = signal(Option::<(bool, String)>::None);
    let (error_message, set_error_message) = signal(Option::<String>::None);
    let is_storage = move || {
        matches!(
            connection_type.get(),
            ConnectionType::S3 | ConnectionType::Gcs | ConnectionType::Azure
        )
    };

    let build_config = move || -> Option<serde_json::Value> {
        let mut config_map = serde_json::Map::new();
        match connection_type.get() {
            ConnectionType::Postgres | ConnectionType::Mysql => {
                if url.get().is_empty() {
                    set_error_message.set(Some("URL is required".to_string()));
                    return None;
                }
                config_map.insert("url".to_string(), serde_json::Value::String(url.get()));
            }
            ConnectionType::HttpApi => {
                if url.get().is_empty() {
                    set_error_message.set(Some("Base URL is required".to_string()));
                    return None;
                }
                config_map.insert("base_url".to_string(), serde_json::Value::String(url.get()));
            }
            ConnectionType::Git => {
                if url.get().is_empty() {
                    set_error_message.set(Some("Repository URL is required".to_string()));
                    return None;
                }
                config_map.insert("repo_url".to_string(), serde_json::Value::String(url.get()));
                if !username.get().is_empty() {
                    config_map.insert(
                        "username".to_string(),
                        serde_json::Value::String(username.get()),
                    );
                }
                if !ssh_key.get().is_empty() {
                    config_map.insert(
                        "ssh_key".to_string(),
                        serde_json::Value::String(ssh_key.get()),
                    );
                }
            }
            ConnectionType::S3 => {
                if bucket.get().is_empty() {
                    set_error_message.set(Some("Bucket is required".to_string()));
                    return None;
                }
                config_map.insert(
                    "bucket".to_string(),
                    serde_json::Value::String(bucket.get()),
                );
                if !region.get().is_empty() {
                    config_map.insert(
                        "region".to_string(),
                        serde_json::Value::String(region.get()),
                    );
                }
                if !endpoint.get().is_empty() {
                    config_map.insert(
                        "endpoint".to_string(),
                        serde_json::Value::String(endpoint.get()),
                    );
                }
                if !access_key.get().is_empty() {
                    config_map.insert(
                        "access_key".to_string(),
                        serde_json::Value::String(access_key.get()),
                    );
                }
                if !secret_key.get().is_empty() {
                    config_map.insert(
                        "secret_key".to_string(),
                        serde_json::Value::String(secret_key.get()),
                    );
                }
                if force_path_style.get() {
                    config_map.insert(
                        "force_path_style".to_string(),
                        serde_json::Value::Bool(true),
                    );
                }
            }
            ConnectionType::Gcs | ConnectionType::Azure => {
                if bucket.get().is_empty() {
                    set_error_message.set(Some("Bucket / Container is required".to_string()));
                    return None;
                }
                config_map.insert(
                    "bucket".to_string(),
                    serde_json::Value::String(bucket.get()),
                );
            }
            ConnectionType::Oci | ConnectionType::Jfrog => {
                if url.get().is_empty() {
                    set_error_message.set(Some("Registry URL is required".to_string()));
                    return None;
                }
                config_map.insert("url".to_string(), serde_json::Value::String(url.get()));
                if !username.get().is_empty() {
                    config_map.insert(
                        "username".to_string(),
                        serde_json::Value::String(username.get()),
                    );
                }
            }
        }
        Some(serde_json::Value::Object(config_map))
    };

    let test_conn = move |_| {
        let config_val = match build_config() {
            Some(v) => v,
            None => return,
        };

        set_is_testing.set(true);
        set_test_result.set(None);
        set_error_message.set(None);

        let c_type = connection_type.get();

        spawn_local(async move {
            match crate::api::test_connection(c_type, config_val).await {
                Ok((success, msg)) => {
                    set_is_testing.set(false);
                    set_test_result.set(Some((success, msg)));
                }
                Err(e) => {
                    set_is_testing.set(false);
                    set_test_result.set(Some((false, e.to_string())));
                }
            }
        });
    };

    let submit = move |_| {
        if name.get().is_empty() {
            set_error_message.set(Some("Name is required".to_string()));
            return;
        }

        let config_val = match build_config() {
            Some(v) => v,
            None => return,
        };

        set_is_submitting.set(true);
        set_error_message.set(None);

        let n = name.get();
        let d = description.get();
        let desc = if d.is_empty() { None } else { Some(d) };
        let c_type = connection_type.get();
        let arn = aws_assume_role_arn.get();
        let a_arn = if arn.is_empty() { None } else { Some(arn) };
        let default_sfs = if is_storage() {
            is_default_sfs.get()
        } else {
            false
        };

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
                                                        <option value="postgres" selected=move || connection_type.get() == ConnectionType::Postgres>"Postgres"</option>
                            <option value="mysql" selected=move || connection_type.get() == ConnectionType::Mysql>"MySQL"</option>
                            <option value="http_api" selected=move || connection_type.get() == ConnectionType::HttpApi>"HTTP API"</option>
                            <option value="git" selected=move || connection_type.get() == ConnectionType::Git>"Git"</option>
                            <option value="s3" selected=move || connection_type.get() == ConnectionType::S3>"S3 / MinIO"</option>
                            <option value="gcs" selected=move || connection_type.get() == ConnectionType::Gcs>"GCS"</option>
                            <option value="azure" selected=move || connection_type.get() == ConnectionType::Azure>"Azure Blob"</option>
                            <option value="oci" selected=move || connection_type.get() == ConnectionType::Oci>"OCI Registry"</option>
                            <option value="jfrog" selected=move || connection_type.get() == ConnectionType::Jfrog>"JFrog"</option>
                        </select>
                    </div>

                    <div style="display: flex; flex-direction: column; gap: 1rem; padding: 1rem; background: var(--surface-background); border: 1px solid var(--surface-border); border-radius: 6px;">
                        <h4 style="margin: 0; color: var(--text-secondary); font-size: 0.9rem; text-transform: uppercase; letter-spacing: 0.5px;">"Connection Configuration"</h4>

                        {move || match connection_type.get() {
                            ConnectionType::Postgres | ConnectionType::Mysql => view! {
                                <div class="form-group">
                                    <label class="form-label">"Database URL *"</label>
                                    <input type="text" class="input-field" placeholder="postgres://user:pass@host/db" prop:value=url on:input=move |ev| set_url.set(event_target_value(&ev)) />
                                </div>
                            }.into_any(),
                            ConnectionType::HttpApi => view! {
                                <div class="form-group">
                                    <label class="form-label">"Base URL *"</label>
                                    <input type="text" class="input-field" placeholder="https://api.example.com" prop:value=url on:input=move |ev| set_url.set(event_target_value(&ev)) />
                                </div>
                            }.into_any(),
                            ConnectionType::Git => view! {
                                <div class="form-group">
                                    <label class="form-label">"Repository URL *"</label>
                                    <input type="text" class="input-field" placeholder="https://github.com/..." prop:value=url on:input=move |ev| set_url.set(event_target_value(&ev)) />
                                </div>
                                <div class="form-group">
                                    <label class="form-label">"Username (Optional)"</label>
                                    <input type="text" class="input-field" placeholder="git" prop:value=username on:input=move |ev| set_username.set(event_target_value(&ev)) />
                                </div>
                                <div class="form-group">
                                    <label class="form-label">"SSH Key (Optional)"</label>
                                    <textarea class="input-field" style="min-height: 80px; font-family: monospace;" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----..." prop:value=ssh_key on:input=move |ev| set_ssh_key.set(event_target_value(&ev)) />
                                </div>
                            }.into_any(),
                            ConnectionType::S3 => view! {
                                <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem;">
                                    <div class="form-group">
                                        <label class="form-label">"Bucket Name *"</label>
                                        <input type="text" class="input-field" placeholder="my-bucket" prop:value=bucket on:input=move |ev| set_bucket.set(event_target_value(&ev)) />
                                    </div>
                                    <div class="form-group">
                                        <label class="form-label">"Region"</label>
                                        <input type="text" class="input-field" placeholder="us-east-1" prop:value=region on:input=move |ev| set_region.set(event_target_value(&ev)) />
                                    </div>
                                    <div class="form-group">
                                        <label class="form-label">"Access Key"</label>
                                        <input type="text" class="input-field" prop:value=access_key on:input=move |ev| set_access_key.set(event_target_value(&ev)) />
                                    </div>
                                    <div class="form-group">
                                        <label class="form-label">"Secret Key"</label>
                                        <input type="password" class="input-field" prop:value=secret_key on:input=move |ev| set_secret_key.set(event_target_value(&ev)) />
                                    </div>
                                </div>
                                <div class="form-group">
                                    <label class="form-label">"Endpoint URL (for MinIO/Custom)"</label>
                                    <input type="text" class="input-field" placeholder="https://s3.mycompany.com" prop:value=endpoint on:input=move |ev| set_endpoint.set(event_target_value(&ev)) />
                                </div>
                                <div class="form-group" style="display: flex; align-items: center; gap: 0.5rem;">
                                    <input type="checkbox" id="force_path_style" prop:checked=force_path_style on:change=move |ev| set_force_path_style.set(event_target_checked(&ev)) />
                                    <label for="force_path_style" class="form-label" style="margin: 0;">"Force Path Style"</label>
                                </div>
                                <div class="form-group">
                                    <label class="form-label">"AWS Assume Role ARN (Optional)"</label>
                                    <input type="text" class="input-field" placeholder="arn:aws:iam::..." prop:value=aws_assume_role_arn on:input=move |ev| set_aws_assume_role_arn.set(event_target_value(&ev)) />
                                </div>
                            }.into_any(),
                            ConnectionType::Gcs | ConnectionType::Azure => view! {
                                <div class="form-group">
                                    <label class="form-label">"Bucket / Container Name *"</label>
                                    <input type="text" class="input-field" placeholder="my-bucket" prop:value=bucket on:input=move |ev| set_bucket.set(event_target_value(&ev)) />
                                </div>
                            }.into_any(),
                            ConnectionType::Oci | ConnectionType::Jfrog => view! {
                                <div class="form-group">
                                    <label class="form-label">"Registry URL *"</label>
                                    <input type="text" class="input-field" placeholder="https://registry.example.com" prop:value=url on:input=move |ev| set_url.set(event_target_value(&ev)) />
                                </div>
                                <div class="form-group">
                                    <label class="form-label">"Username (Optional)"</label>
                                    <input type="text" class="input-field" prop:value=username on:input=move |ev| set_username.set(event_target_value(&ev)) />
                                </div>
                            }.into_any(),
                        }}
                    </div>

                    {move || if is_storage() {
                        view! {
                            <div class="form-group" style="display: flex; align-items: center; gap: 0.5rem; padding: 1rem; background: var(--surface-elevated); border-radius: 6px; border: 1px dashed var(--primary-color);">
                                <input type="checkbox" id="is_default_sfs" prop:checked=is_default_sfs on:change=move |ev| set_is_default_sfs.set(event_target_checked(&ev)) />
                                <label for="is_default_sfs" class="form-label" style="margin: 0; font-weight: 500; color: var(--primary-color);">"Use as default SFS backend"</label>
                            </div>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                </div>

                <div class="modal-footer" style="display: flex; justify-content: flex-end; gap: 1rem; padding: 1.5rem; border-top: 1px solid var(--surface-border); flex-wrap: wrap;">
                    {move || test_result.get().map(|(success, msg)| view! {
                        <div style=format!("padding: 1rem; border-radius: 6px; font-weight: 500; font-size: 0.9rem; flex-basis: 100%; margin-bottom: 1rem; {}", if success { "background: rgba(34, 197, 94, 0.1); border: 1px solid rgba(34, 197, 94, 0.2); color: var(--status-success);" } else { "background: rgba(239, 68, 68, 0.1); border: 1px solid rgba(239, 68, 68, 0.2); color: var(--status-error);" })>
                            {if success { "✅ " } else { "❌ " }} {msg}
                        </div>
                    })}
                    <div style="display: flex; gap: 1rem; align-items: center;">
                        <button class="btn-secondary" on:click=test_conn disabled=move || is_testing.get() || is_submitting.get()>
                            {move || if is_testing.get() { "Testing..." } else { "Test Connection" }}
                        </button>
                        <button class="btn-secondary" on:click=move |_| on_close.run(()) disabled=move || is_submitting.get()>
                            "Cancel"
                        </button>
                        <button class="btn-primary" on:click=submit disabled=move || is_submitting.get() || is_testing.get()>
                            {move || if is_submitting.get() { "Creating..." } else { "Create Connection" }}
                        </button>
                    </div>
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
                            <ConnectionsTable connections=connections on_delete_success=Callback::new(move |_| connections_resource.refetch()) />
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
