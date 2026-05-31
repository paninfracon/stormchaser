use crate::api::{parse_dsl, ParseDslResult};
use leptos::prelude::*;
use leptos::task::spawn_local;

mod schema_form;
use schema_form::SchemaForm;

#[component]
pub fn CreateRunModal(on_close: Callback<()>, on_success: Callback<()>) -> impl IntoView {
    let (active_tab, set_active_tab) = signal("git".to_string());

    view! {
        <div class="modal-overlay" style="position: fixed; top: 0; left: 0; right: 0; bottom: 0; background: rgba(0, 0, 0, 0.7); display: flex; align-items: center; justify-content: center; z-index: 1000;">
            <div class="modal-content glass-panel" style="width: 800px; max-width: 90vw; max-height: 90vh; display: flex; flex-direction: column;">
                <div style="display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border); padding: 1rem 1.5rem;">
                    <h2 style="margin: 0; font-size: 1.25rem;">"Create Workflow Run"</h2>
                    <button style="background: transparent; border: none; color: var(--text-secondary); cursor: pointer; font-size: 1.5rem;" on:click=move |_| on_close.run(())>
                        "×"
                    </button>
                </div>

                <div style="display: flex; border-bottom: 1px solid var(--surface-border); padding: 0 1.5rem;">
                    <button
                        style=move || format!("padding: 1rem 1.5rem; background: transparent; border: none; border-bottom: 2px solid {}; color: {}; cursor: pointer; font-weight: 500;", if active_tab.get() == "git" { "var(--primary-color)" } else { "transparent" }, if active_tab.get() == "git" { "var(--text-primary)" } else { "var(--text-secondary)" })
                        on:click=move |_| set_active_tab.set("git".to_string())
                    >
                        "From Git"
                    </button>
                    <button
                        style=move || format!("padding: 1rem 1.5rem; background: transparent; border: none; border-bottom: 2px solid {}; color: {}; cursor: pointer; font-weight: 500;", if active_tab.get() == "direct" { "var(--primary-color)" } else { "transparent" }, if active_tab.get() == "direct" { "var(--text-primary)" } else { "var(--text-secondary)" })
                        on:click=move |_| set_active_tab.set("direct".to_string())
                    >
                        "Direct Input"
                    </button>
                </div>

                <div style="padding: 1.5rem; flex: 1; overflow-y: auto;">
                    {move || match active_tab.get().as_str() {
                        "git" => view! { <GitRunForm on_success=on_success /> }.into_any(),
                        "direct" => view! { <DirectRunForm on_success=on_success /> }.into_any(),
                        _ => ().into_any()
                    }}
                </div>
            </div>
        </div>
    }
}

#[component]
fn GitRunForm(on_success: Callback<()>) -> impl IntoView {
    let connections_res = Resource::new(
        || (),
        |_| async { crate::api::fetch_connections().await.unwrap_or_default() },
    );

    let (selected_connection, set_selected_connection) = signal(String::new());
    let (workflow_path, set_workflow_path) = signal(String::new());
    let (git_ref, set_git_ref) = signal(String::new());

    let (parsed_data, set_parsed_data) = signal(Option::<ParseDslResult>::None);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (is_loading, set_is_loading) = signal(false);

    let load_form = move |_| {
        if selected_connection.get().is_empty()
            || workflow_path.get().is_empty()
            || git_ref.get().is_empty()
        {
            set_error_msg.set(Some("All fields are required.".to_string()));
            return;
        }

        set_is_loading.set(true);
        set_error_msg.set(None);

        let conn = selected_connection.get();
        let path = workflow_path.get();
        let reference = git_ref.get();

        spawn_local(async move {
            match crate::api::fetch_dsl_from_git(conn.clone(), path, reference).await {
                Ok(dsl_content) => match parse_dsl(dsl_content).await {
                    Ok(res) => set_parsed_data.set(Some(res)),
                    Err(e) => set_error_msg.set(Some(format!("Failed to parse DSL: {}", e))),
                },
                Err(e) => set_error_msg.set(Some(e.to_string())),
            }
            set_is_loading.set(false);
        });
    };

    let submit = move |inputs: serde_json::Value| {
        let conn = selected_connection.get();
        let path = workflow_path.get();
        let reference = git_ref.get();

        spawn_local(async move {
            match crate::api::submit_run_git(conn, path, reference, inputs).await {
                Ok(_) => on_success.run(()),
                Err(e) => set_error_msg.set(Some(e.to_string())),
            }
        });
    };

    view! {
        <div style="display: flex; flex-direction: column; gap: 1rem; height: 100%;">
            {move || if parsed_data.get().is_none() {
                view! {
                    <div style="display: flex; flex-direction: column; gap: 1rem; flex: 1;">
                        {move || error_msg.get().map(|msg| view! {
                            <div style="padding: 1rem; background: rgba(255, 59, 48, 0.1); color: var(--status-error); border-radius: 8px; border: 1px solid var(--status-error);">
                                {msg}
                            </div>
                        })}
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label>"Git Connection"</label>
                            <Suspense fallback=|| view! { <div>"Loading connections..."</div> }>
                                {move || {
                                    let conns = connections_res.get().unwrap_or_default();
                                    let git_conns: Vec<_> = conns.into_iter().filter(|c| matches!(c.connection_type, crate::models::ConnectionType::Git)).collect();

                                    view! {
                                        <select
                                            class="input-field"
                                            style="padding: 0.75rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); width: 100%;"
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
                                            }).collect::<Vec<_>>()}
                                        </select>
                                    }
                                }}
                            </Suspense>
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label>"Workflow Path"</label>
                            <input type="text" placeholder=".stormchaser/workflows/main.storm" class="input-field" style="padding: 0.75rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); width: 100%;" on:input=move |ev| set_workflow_path.set(event_target_value(&ev)) />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label>"Git Ref (Branch/Tag/Commit)"</label>
                            <input type="text" placeholder="main" class="input-field" style="padding: 0.75rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); width: 100%;" on:input=move |ev| set_git_ref.set(event_target_value(&ev)) />
                        </div>

                        <div style="display: flex; justify-content: flex-end; margin-top: 1rem;">
                            <button
                                style="padding: 0.75rem 1.5rem; border-radius: 4px; border: none; background: var(--primary-color); color: white; cursor: pointer; font-weight: 500;"
                                disabled=move || is_loading.get()
                                on:click=load_form
                            >
                                {move || if is_loading.get() { "Loading Form..." } else { "Load Form" }}
                            </button>
                        </div>
                    </div>
                }.into_any()
            } else {
                let pd = parsed_data.get().unwrap();
                view! {
                    <div style="display: flex; flex-direction: column; gap: 1rem; flex: 1; min-height: 0;">
                        <div style="display: flex; justify-content: space-between; align-items: center;">
                            <h3 style="margin: 0;">"Configure Run"</h3>
                            <button
                                style="background: none; border: none; color: var(--text-secondary); cursor: pointer; padding: 0.5rem;"
                                on:click=move |_| set_parsed_data.set(None)
                            >
                                "← Back"
                            </button>
                        </div>
                        {move || error_msg.get().map(|msg| view! {
                            <div style="padding: 1rem; background: rgba(255, 59, 48, 0.1); color: var(--status-error); border-radius: 8px; border: 1px solid var(--status-error);">
                                {msg}
                            </div>
                        })}
                        <div style="flex: 1; overflow-y: auto; padding-right: 0.5rem;" class="custom-scrollbar">
                            <SchemaForm
                                base_schema=pd.inputs_schema
                                initial_inputs=pd.inputs
                                queries=pd.queries
                                inputs_view=pd.inputs_view
                                on_submit=Callback::new(submit)
                                on_back=Callback::new(move |_| set_parsed_data.set(None))
                            />
                        </div>
                    </div>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn DirectRunForm(on_success: Callback<()>) -> impl IntoView {
    let (dsl, set_dsl) = signal(String::new());
    let (parsed_data, set_parsed_data) = signal(Option::<ParseDslResult>::None);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (is_parsing, set_is_parsing) = signal(false);

    let parse = move |_| {
        if dsl.get().is_empty() {
            set_error_msg.set(Some("DSL cannot be empty.".to_string()));
            return;
        }

        set_is_parsing.set(true);
        set_error_msg.set(None);

        let dsl_content = dsl.get();
        spawn_local(async move {
            match parse_dsl(dsl_content).await {
                Ok(res) => set_parsed_data.set(Some(res)),
                Err(e) => set_error_msg.set(Some(e.to_string())),
            }
            set_is_parsing.set(false);
        });
    };

    view! {
        <div style="display: flex; flex-direction: column; gap: 1rem; height: 100%;">
            {move || if parsed_data.get().is_none() {
                view! {
                    <div style="display: flex; flex-direction: column; gap: 1rem; flex: 1;">
                        {move || error_msg.get().map(|msg| view! {
                            <div style="padding: 1rem; background: rgba(255, 59, 48, 0.1); color: var(--status-error); border-radius: 8px; border: 1px solid var(--status-error);">
                                {msg}
                            </div>
                        })}
                        <div style="display: flex; flex-direction: column; gap: 0.5rem; flex: 1;">
                            <label>"Workflow DSL"</label>
                            <textarea
                                placeholder="Paste your .storm file contents here..."
                                style="padding: 0.75rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); width: 100%; min-height: 300px; font-family: monospace; flex: 1; resize: vertical;"
                                on:input=move |ev| set_dsl.set(event_target_value(&ev))
                            ></textarea>
                        </div>
                        <div style="display: flex; justify-content: flex-end; margin-top: 1rem;">
                            <button
                                style="padding: 0.75rem 1.5rem; border-radius: 4px; border: none; background: var(--primary-color); color: white; cursor: pointer; font-weight: 500;"
                                disabled=move || is_parsing.get()
                                on:click=parse
                            >
                                {move || if is_parsing.get() { "Parsing..." } else { "Next" }}
                            </button>
                        </div>
                    </div>
                }.into_any()
            } else {
                let data = parsed_data.get().unwrap();
                view! {
                    <SchemaForm
                        base_schema=data.inputs_schema.clone()
                        initial_inputs=data.inputs.clone()
                        queries=data.queries.clone()
                        inputs_view=data.inputs_view.clone()
                        on_submit=Callback::new(move |final_inputs| {
                            let current_dsl = dsl.get();
                            spawn_local(async move {
                                match crate::api::submit_run_direct(current_dsl, final_inputs).await {
                                    Ok(_) => on_success.run(()),
                                    Err(e) => set_error_msg.set(Some(e.to_string())),
                                }
                            });
                        })
                        on_back=Callback::new(move |_| set_parsed_data.set(None))
                    />
                }.into_any()
            }}
        </div>
    }
}
