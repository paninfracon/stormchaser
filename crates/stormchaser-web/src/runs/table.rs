use crate::models::{RunStatus, WorkflowRunDetail};
use leptos::prelude::*;

#[component]
pub fn RunsTable(
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
