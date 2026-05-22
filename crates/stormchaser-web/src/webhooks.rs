use crate::api::fetch_webhooks;
use crate::models::WebhookConfig;
use leptos::prelude::*;

#[component]
pub fn WebhooksTable(webhooks: Vec<WebhookConfig>) -> impl IntoView {
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
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

#[component]
pub fn WebhooksList() -> impl IntoView {
    let webhooks_resource = Resource::new(|| (), |_| async { fetch_webhooks().await });

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            <div class="glass-panel table-container" style="flex: 1; overflow: auto;">
                <div style="padding: 1.5rem; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem; font-weight: 600;">"Webhooks"</h2>
                </div>

                <Suspense fallback=|| view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading webhooks..."</div> }>
                    {move || match webhooks_resource.get() {
                        Some(Ok(webhooks)) => view! {
                            <WebhooksTable webhooks=webhooks />
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
