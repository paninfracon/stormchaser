use crate::api::{
    hydrate_schema_complete, parse_dsl, submit_run_direct, submit_run_git, ParseDslResult,
};
use leptos::prelude::*;
use leptos::task::spawn_local;

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
    let (repo_url, set_repo_url) = signal(String::new());
    let (workflow_path, set_workflow_path) = signal(String::new());
    let (git_ref, set_git_ref) = signal(String::new());
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (is_submitting, set_is_submitting) = signal(false);

    let submit = move |_| {
        if repo_url.get().is_empty() || workflow_path.get().is_empty() || git_ref.get().is_empty() {
            set_error_msg.set(Some("All fields are required.".to_string()));
            return;
        }

        set_is_submitting.set(true);
        set_error_msg.set(None);

        let url = repo_url.get();
        let path = workflow_path.get();
        let reference = git_ref.get();

        spawn_local(async move {
            match submit_run_git(url, path, reference).await {
                Ok(_) => on_success.run(()),
                Err(e) => set_error_msg.set(Some(e.to_string())),
            }
            set_is_submitting.set(false);
        });
    };

    view! {
        <div style="display: flex; flex-direction: column; gap: 1rem;">
            {move || error_msg.get().map(|msg| view! {
                <div style="padding: 1rem; background: rgba(255, 59, 48, 0.1); color: var(--status-error); border-radius: 8px; border: 1px solid var(--status-error);">
                    {msg}
                </div>
            })}
            <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                <label>"Repository URL"</label>
                <input type="text" placeholder="https://github.com/..." class="input-field" style="padding: 0.75rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); width: 100%;" on:input=move |ev| set_repo_url.set(event_target_value(&ev)) />
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
                    disabled=move || is_submitting.get()
                    on:click=submit
                >
                    {move || if is_submitting.get() { "Submitting..." } else { "Create Run" }}
                </button>
            </div>
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
                        dsl=dsl
                        base_schema=data.inputs_schema.clone()
                        initial_inputs=data.inputs.clone()
                        queries=data.queries.clone()
                        inputs_view=data.inputs_view.clone()
                        on_success=on_success
                        on_back=Callback::new(move |_| set_parsed_data.set(None))
                    />
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn SchemaForm(
    #[prop(into)] dsl: Signal<String>,
    base_schema: Option<serde_json::Value>,
    initial_inputs: serde_json::Value,
    queries: Option<serde_json::Value>,
    inputs_view: Option<stormchaser_model::dsl::InputView>,
    on_success: Callback<()>,
    on_back: Callback<()>,
) -> impl IntoView {
    let (inputs, set_inputs) = signal(initial_inputs.clone());
    let (hydrated_schema, set_hydrated_schema) = signal(
        base_schema
            .clone()
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
    );
    let (hydration_status, set_hydration_status) = signal("Ready".to_string());
    let (validation_errors, set_validation_errors) = signal(Vec::<String>::new());
    let (is_submitting, set_is_submitting) = signal(false);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);

    // Auto-hydrate on mount
    let schema_clone = base_schema.clone();
    let queries_clone = queries.clone();

    Effect::new(move |_| {
        let current_inputs = inputs.get();
        let schema_val = schema_clone.clone();
        let q_val = queries_clone.clone();

        spawn_local(async move {
            set_hydration_status.set("Hydrating...".to_string());
            if let Some(schema) = schema_val {
                match hydrate_schema_complete(schema, current_inputs, q_val).await {
                    Ok((new_schema, status, errors)) => {
                        set_hydrated_schema.set(new_schema);
                        set_hydration_status.set(status);
                        set_validation_errors.set(errors);
                    }
                    Err(e) => {
                        set_hydration_status.set("Failed".to_string());
                        set_validation_errors.set(vec![e.to_string()]);
                    }
                }
            } else {
                set_hydration_status.set("Ready".to_string());
            }
        });
    });

    let submit = move |_| {
        set_is_submitting.set(true);
        set_error_msg.set(None);

        let current_dsl = dsl.get();
        let current_inputs = inputs.get();

        spawn_local(async move {
            match submit_run_direct(current_dsl, current_inputs).await {
                Ok(_) => on_success.run(()),
                Err(e) => set_error_msg.set(Some(e.to_string())),
            }
            set_is_submitting.set(false);
        });
    };

    let update_input = move |key: String, value: String| {
        set_inputs.update(|current| {
            if let serde_json::Value::Object(map) = current {
                map.insert(key, serde_json::Value::String(value));
            }
        });
    };

    view! {
        <div style="display: flex; flex-direction: column; gap: 1rem; height: 100%;">
            {move || error_msg.get().map(|msg| view! {
                <div style="padding: 1rem; background: rgba(255, 59, 48, 0.1); color: var(--status-error); border-radius: 8px; border: 1px solid var(--status-error);">
                    {msg}
                </div>
            })}

            <div style="display: flex; justify-content: space-between; align-items: center; background: var(--surface-elevated); padding: 0.75rem 1rem; border-radius: 4px; border: 1px solid var(--surface-border);">
                <span style="font-weight: 500;">"Workflow Inputs"</span>
                <span style=move || format!("font-size: 0.85rem; padding: 0.25rem 0.5rem; border-radius: 4px; background: rgba(0, 122, 255, 0.1); color: {};", if hydration_status.get() == "Completed" { "var(--status-succeeded)" } else { "var(--primary-color)" })>
                    {move || hydration_status.get()}
                </span>
            </div>

            {move || {
                let errs = validation_errors.get();
                if !errs.is_empty() {
                    view! {
                        <div style="padding: 1rem; background: rgba(255, 59, 48, 0.1); color: var(--status-error); border-radius: 8px; border: 1px solid var(--status-error); font-size: 0.85rem;">
                            <ul style="margin: 0; padding-left: 1rem;">
                                {errs.into_iter().map(|e| view! { <li>{e}</li> }).collect_view()}
                            </ul>
                        </div>
                    }.into_any()
                } else {
                    ().into_any()
                }
            }}

            <div style="display: flex; flex-direction: column; gap: 1rem; flex: 1; overflow-y: auto; padding-right: 0.5rem;">
                {move || {
                    let schema = hydrated_schema.get();
                    let current_inputs = inputs.get();

                    let mut fields = Vec::new();
                    let required_fields: Vec<String> = schema.get("required")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                        .unwrap_or_default();

                    if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
                        for (key, prop) in properties {
                            let description = prop.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let is_required = required_fields.contains(key);
                            let default_val = prop.get("default").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let current_val = current_inputs.get(key).and_then(|v| v.as_str()).unwrap_or(&default_val).to_string();

                            let mut options = Vec::new();
                            if let Some(enum_vals) = prop.get("enum").and_then(|v| v.as_array()) {
                                for v in enum_vals {
                                    if let Some(s) = v.as_str() { options.push(s.to_string()); }
                                    else { options.push(v.to_string()); }
                                }
                            }

                            fields.push((key.clone(), description, is_required, current_val, options));
                        }
                    }

                    if let Some(view_rules) = &inputs_view {
                        let mut ordered_fields = Vec::new();
                        let mut remaining_fields = fields.clone();
                        let has_wildcard = view_rules.ui_order.contains(&"*".to_string());

                        for order_key in &view_rules.ui_order {
                            if order_key == "*" {
                                ordered_fields.append(&mut remaining_fields);
                            } else if let Some(idx) = remaining_fields.iter().position(|(k, _, _, _, _)| k == order_key) {
                                ordered_fields.push(remaining_fields.remove(idx));
                            }
                        }

                        if !has_wildcard {
                            ordered_fields.append(&mut remaining_fields);
                        }
                        fields = ordered_fields;
                    }

                    if fields.is_empty() {
                        view! {
                            <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">
                                "This workflow does not require any inputs."
                            </div>
                        }.into_any()
                    } else {
                        fields.into_iter().map(|(key, desc, req, val, opts)| {
                            let key_clone = key.clone();

                            view! {
                                <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                                    <label style="font-size: 0.85rem; font-weight: 500;">
                                        {key.clone()} {if req { "*" } else { "" }}
                                    </label>
                                    {if !desc.is_empty() {
                                        view! { <span style="font-size: 0.75rem; color: var(--text-secondary); margin-bottom: 0.25rem;">{desc}</span> }.into_any()
                                    } else {
                                        ().into_any()
                                    }}

                                    {if !opts.is_empty() {
                                        let key_clone_select = key_clone.clone();
                                        view! {
                                            <select
                                                class="input-field"
                                                style="padding: 0.75rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); width: 100%;"
                                                on:change=move |ev| update_input(key_clone_select.clone(), event_target_value(&ev))
                                            >
                                                <option value="" disabled=true selected=val.is_empty()>"Select an option"</option>
                                                {opts.into_iter().map(|o| {
                                                    let is_selected = o == val;
                                                    view! { <option value=o.clone() selected=is_selected>{o.clone()}</option> }
                                                }).collect_view()}
                                            </select>
                                        }.into_any()
                                    } else {
                                        let key_clone_input = key_clone.clone();
                                        view! {
                                            <input
                                                type="text"
                                                class="input-field"
                                                style="padding: 0.75rem; border-radius: 4px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); width: 100%;"
                                                prop:value=val
                                                on:change=move |ev| update_input(key_clone_input.clone(), event_target_value(&ev))
                                            />
                                        }.into_any()
                                    }}
                                </div>
                            }
                        }).collect_view().into_any()
                    }
                }}
            </div>

            <div style="display: flex; justify-content: space-between; margin-top: 1rem; padding-top: 1rem; border-top: 1px solid var(--surface-border);">
                <button
                    style="padding: 0.75rem 1.5rem; border-radius: 4px; border: 1px solid var(--surface-border); background: transparent; color: var(--text-primary); cursor: pointer; font-weight: 500;"
                    on:click=move |_| on_back.run(())
                >
                    "Back"
                </button>
                <button
                    style="padding: 0.75rem 1.5rem; border-radius: 4px; border: none; background: var(--primary-color); color: white; cursor: pointer; font-weight: 500;"
                    disabled=move || is_submitting.get() || hydration_status.get() != "Completed"
                    on:click=submit
                >
                    {move || if is_submitting.get() { "Submitting..." } else { "Create Run" }}
                </button>
            </div>
        </div>
    }
}
