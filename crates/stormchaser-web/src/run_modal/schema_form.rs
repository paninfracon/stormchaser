use crate::api::hydrate_schema_complete;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn SchemaForm(
    base_schema: Option<serde_json::Value>,
    initial_inputs: serde_json::Value,
    queries: Option<serde_json::Value>,
    inputs_view: Option<stormchaser_model::dsl::InputView>,
    on_submit: Callback<serde_json::Value>,
    on_back: Callback<()>,
) -> impl IntoView {
    let (inputs, set_inputs) = signal(initial_inputs.clone());
    let (hydrated_schema, set_hydrated_schema) = signal(
        base_schema
            .clone()
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
    );
    let (hydration_status, set_hydration_status) = signal("Pending".to_string());
    let (validation_errors, set_validation_errors) = signal(Vec::<String>::new());

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

    let submit = move |_: leptos::ev::MouseEvent| {
        let current_inputs = inputs.get();
        on_submit.run(current_inputs);
    };

    let update_input = move |(key, value): (String, String)| {
        set_inputs.update(|current| {
            if let serde_json::Value::Object(map) = current {
                map.insert(key, serde_json::Value::String(value));
            }
        });
    };

    view! {
        <div style="display: flex; flex-direction: column; gap: 1rem; height: 100%;">

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

                    view! {
                        <SchemaFieldsList
                            schema=schema
                            current_inputs=current_inputs
                            inputs_view=inputs_view.clone()
                            on_update_input=Callback::new(update_input)
                        />
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
                    disabled=move || hydration_status.get() != "Completed"
                    on:click=submit
                >
                    "Create Run"
                </button>
            </div>
        </div>
    }
}

#[component]
fn SchemaFieldsList(
    schema: serde_json::Value,
    current_inputs: serde_json::Value,
    inputs_view: Option<stormchaser_model::dsl::InputView>,
    on_update_input: Callback<(String, String)>,
) -> impl IntoView {
    let mut fields = Vec::new();
    let required_fields: Vec<String> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
        for (key, prop) in properties {
            let description = prop
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_required = required_fields.contains(key);
            let default_val = prop
                .get("default")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let current_val = current_inputs
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or(&default_val)
                .to_string();

            let mut options = Vec::new();
            if let Some(enum_vals) = prop.get("enum").and_then(|v| v.as_array()) {
                for v in enum_vals {
                    if let Some(s) = v.as_str() {
                        options.push(s.to_string());
                    } else {
                        options.push(v.to_string());
                    }
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
            } else if let Some(idx) = remaining_fields
                .iter()
                .position(|(k, _, _, _, _)| k == order_key)
            {
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
        }
        .into_any()
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
                                on:change=move |ev| on_update_input.run((key_clone_select.clone(), event_target_value(&ev)))
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
                                on:change=move |ev| on_update_input.run((key_clone_input.clone(), event_target_value(&ev)))
                            />
                        }.into_any()
                    }}
                </div>
            }
        }).collect_view().into_any()
    }
}
