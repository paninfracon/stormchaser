use crate::api::fetch_event_rules;
use crate::models::EventRule;
use leptos::prelude::*;

#[component]
pub fn RulesTable(rules: Vec<EventRule>) -> impl IntoView {
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
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

#[component]
pub fn RulesList() -> impl IntoView {
    let rules_resource = Resource::new(|| (), |_| async { fetch_event_rules().await });

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            <div class="glass-panel table-container" style="flex: 1; overflow: auto;">
                <div style="padding: 1.5rem; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem; font-weight: 600;">"Event Rules"</h2>
                </div>

                <Suspense fallback=|| view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading event rules..."</div> }>
                    {move || match rules_resource.get() {
                        Some(Ok(rules)) => view! {
                            <RulesTable rules=rules />
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
