use crate::models::{RunStatus, WorkflowRunDetail};
use leptos::prelude::*;

#[component]
pub fn RunsTable(
    #[prop(into)] runs: Signal<Vec<WorkflowRunDetail>>,
    #[prop(into)] selected_run_id: Signal<Option<String>>,
    set_selected_run_id: WriteSignal<Option<String>>,
) -> impl IntoView {
    let (sort_column, set_sort_column) = signal("Created".to_string());
    let (sort_desc, set_sort_desc) = signal(true);

    let sorted_runs = Memo::new(move |_| {
        let mut r = runs.get();
        let col = sort_column.get();
        let desc = sort_desc.get();
        r.sort_by(|a, b| {
            let cmp = match col.as_str() {
                "Name" => a.workflow_name.cmp(&b.workflow_name),
                "Initiator" => a.initiating_user.cmp(&b.initiating_user),
                "Status" => String::from(a.status.clone()).cmp(&String::from(b.status.clone())),
                "Created" => a.created_at.cmp(&b.created_at),
                "Finished" => a.finished_at.cmp(&b.finished_at),
                _ => std::cmp::Ordering::Equal,
            };
            if desc {
                cmp.reverse()
            } else {
                cmp
            }
        });
        r
    });

    let header_cell = move |label: &'static str| {
        view! {
            <th
                style="cursor: pointer; user-select: none;"
                on:click=move |_| {
                    if sort_column.get() == label {
                        set_sort_desc.update(|d| *d = !*d);
                    } else {
                        set_sort_column.set(label.to_string());
                        set_sort_desc.set(false); // default to ascending when changing column
                    }
                }
            >
                {label}
                {move || if sort_column.get() == label {
                    if sort_desc.get() { " ▼" } else { " ▲" }
                } else {
                    ""
                }}
            </th>
        }
    };

    view! {
        <table>
            <thead>
                <tr>
                    {header_cell("Name")}
                    {header_cell("Initiator")}
                    {header_cell("Status")}
                    {header_cell("Created")}
                    {header_cell("Finished")}
                </tr>
            </thead>
            <tbody>
                {move || sorted_runs.get().into_iter().map(|run| {
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
