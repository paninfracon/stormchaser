use crate::api::fetch_workflow_runs;
use crate::runs::{details::RunDetailsPanel, table::RunsTable};
use leptos::prelude::*;

#[component]
pub fn ApprovalsList() -> impl IntoView {
    // We only fetch runs that are "Running"
    let runs_resource = Resource::new(
        || (),
        |_| async move {
            fetch_workflow_runs(
                Some("running".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
        },
    );

    let (selected_run_id, set_selected_run_id) = signal(None::<String>);
    let (selected_step_id, set_selected_step_id) = signal(None::<String>);

    let _run_change_effect = Effect::new(move |_| {
        selected_run_id.get();
        set_selected_step_id.set(None);
    });

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            <div class="glass-panel table-container" style=move || if selected_run_id.get().is_some() { "flex: 1; overflow: hidden; max-width: 50%; display: flex; flex-direction: column;" } else { "flex: 1; overflow: hidden; display: flex; flex-direction: column;" }>
                <div style="display: flex; justify-content: space-between; align-items: center; padding: 1rem; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem;">"Pending Approvals"</h2>
                </div>
                <div style="flex: 1; overflow: auto;">
                    <Suspense fallback=move || view! { <div style="padding: 2rem; text-align: center;">"Loading..."</div> }>
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
                                                "No pending approvals found."
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! { <RunsTable runs=runs_list selected_run_id=selected_run_id set_selected_run_id=set_selected_run_id /> }.into_any()
                                    }
                                }
                                Some(Err(e)) => {
                                    view! {
                                        <div style="padding: 2rem; text-align: center; color: var(--status-error);">
                                            {format!("Failed to load approvals: {}", e)}
                                        </div>
                                    }.into_any()
                                }
                                None => ().into_any()
                            }
                        }
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
