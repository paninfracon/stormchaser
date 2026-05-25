use leptos::prelude::*;

#[component]
pub fn RunsFilterPanel(
    set_filter_owner: WriteSignal<Option<String>>,
    set_filter_name: WriteSignal<Option<String>>,
    set_filter_repo_url: WriteSignal<Option<String>>,
    set_filter_workflow_path: WriteSignal<Option<String>>,
    set_filter_created_after: WriteSignal<Option<String>>,
    set_filter_created_before: WriteSignal<Option<String>>,
    set_filter_status: WriteSignal<Option<String>>,
    #[prop(into)] current_status: Signal<Option<String>>,
    #[prop(into)] current_owner: Signal<Option<String>>,
    #[prop(into)] current_name: Signal<Option<String>>,
    #[prop(into)] current_repo_url: Signal<Option<String>>,
    #[prop(into)] current_workflow_path: Signal<Option<String>>,
    #[prop(into)] current_created_after: Signal<Option<String>>,
    #[prop(into)] current_created_before: Signal<Option<String>>,
) -> impl IntoView {
    let (is_expanded, set_is_expanded) = signal(false);

    // Local state for inputs
    let (local_owner, set_local_owner) = signal(current_owner.get_untracked().unwrap_or_default());
    let (local_name, set_local_name) = signal(current_name.get_untracked().unwrap_or_default());
    let (local_repo_url, set_local_repo_url) =
        signal(current_repo_url.get_untracked().unwrap_or_default());
    let (local_workflow_path, set_local_workflow_path) =
        signal(current_workflow_path.get_untracked().unwrap_or_default());
    let (local_created_after, set_local_created_after) =
        signal(current_created_after.get_untracked().unwrap_or_default());
    let (local_created_before, set_local_created_before) =
        signal(current_created_before.get_untracked().unwrap_or_default());
    let (local_status, set_local_status) = signal(
        current_status
            .get_untracked()
            .unwrap_or_else(|| "Any".to_string()),
    );

    let apply_filters = move |_| {
        let o = local_owner.get();
        let n = local_name.get();
        let r = local_repo_url.get();
        let w = local_workflow_path.get();
        let ca = local_created_after.get();
        let cb = local_created_before.get();
        let s = local_status.get();

        set_filter_owner.set(if o.is_empty() { None } else { Some(o) });
        set_filter_name.set(if n.is_empty() { None } else { Some(n) });
        set_filter_repo_url.set(if r.is_empty() { None } else { Some(r) });
        set_filter_workflow_path.set(if w.is_empty() { None } else { Some(w) });
        set_filter_created_after.set(if ca.is_empty() { None } else { Some(ca) });
        set_filter_created_before.set(if cb.is_empty() { None } else { Some(cb) });
        set_filter_status.set(if s.is_empty() { None } else { Some(s) });
    };

    let clear_filters = move |_| {
        set_local_owner.set(String::new());
        set_local_name.set(String::new());
        set_local_repo_url.set(String::new());
        set_local_workflow_path.set(String::new());
        set_local_created_after.set(String::new());
        set_local_created_before.set(String::new());
        set_local_status.set("Any".to_string());

        set_filter_owner.set(None);
        set_filter_name.set(None);
        set_filter_repo_url.set(None);
        set_filter_workflow_path.set(None);
        set_filter_created_after.set(None);
        set_filter_created_before.set(None);
        set_filter_status.set(Some("Any".to_string()));
    };

    let statuses = vec![
        "Any",
        "queued",
        "resolving",
        "start_pending",
        "running",
        "succeeded",
        "failed",
    ];

    view! {
        <div style="border-bottom: 1px solid var(--surface-border); padding: 1rem; flex: none;">
            <div style="display: flex; justify-content: space-between; align-items: center; cursor: pointer; user-select: none;" on:click=move |_| set_is_expanded.update(|e| *e = !*e)>
                <h3 style="margin: 0; display: flex; align-items: center; gap: 0.5rem; font-size: 1rem;">
                    "Filters"
                </h3>
                <span>{move || if is_expanded.get() { "▲" } else { "▼" }}</span>
            </div>

            <div style=move || format!("display: {}; margin-top: 1rem;", if is_expanded.get() { "block" } else { "none" })>
                <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 1rem; margin-bottom: 1rem;">
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Owner"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_owner on:input=move |ev| set_local_owner.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Name"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_name on:input=move |ev| set_local_name.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Repo URL"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_repo_url on:input=move |ev| set_local_repo_url.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Workflow Path"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" prop:value=local_workflow_path on:input=move |ev| set_local_workflow_path.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Status"</label>
                        <select style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" on:change=move |ev| set_local_status.set(event_target_value(&ev))>
                            {statuses.into_iter().map(|s| {
                                let s_clone = s.to_string();
                                view! {
                                    <option value=s selected=move || local_status.get() == s_clone style="background: var(--surface-elevated); color: var(--text-primary);">
                                        {s}
                                    </option>
                                }
                            }).collect_view()}
                        </select>
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Created After"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" placeholder="e.g. 2024-01-01T00:00:00Z" prop:value=local_created_after on:input=move |ev| set_local_created_after.set(event_target_value(&ev)) />
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                        <label style="font-size: 0.85rem; color: var(--text-secondary);">"Created Before"</label>
                        <input type="text" style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary);" placeholder="e.g. 2024-01-01T00:00:00Z" prop:value=local_created_before on:input=move |ev| set_local_created_before.set(event_target_value(&ev)) />
                    </div>
                </div>

                <div style="display: flex; justify-content: flex-end; gap: 0.5rem;">
                    <button style="padding: 0.5rem 1rem; border-radius: 4px; border: 1px solid var(--surface-border); background: transparent; color: var(--text-primary); cursor: pointer;" on:click=clear_filters>"Clear"</button>
                    <button style="padding: 0.5rem 1rem; border-radius: 4px; border: none; background: var(--primary-color); color: white; cursor: pointer; font-weight: 500;" on:click=apply_filters>"Apply Filters"</button>
                </div>
            </div>
        </div>
    }
}
