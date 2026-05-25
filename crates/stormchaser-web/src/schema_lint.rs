use crate::api::{parse_dsl, ParseDslResult};
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn SchemaLinter() -> impl IntoView {
    let (dsl, set_dsl) = signal(String::new());
    let (parsed_data, set_parsed_data) = signal(Option::<ParseDslResult>::None);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (is_parsing, set_is_parsing) = signal(false);

    let parse = move |_| {
        if dsl.get().is_empty() {
            set_error_msg.set(Some("DSL cannot be empty.".to_string()));
            set_parsed_data.set(None);
            return;
        }

        set_is_parsing.set(true);
        set_error_msg.set(None);
        set_parsed_data.set(None);

        let dsl_content = dsl.get();
        spawn_local(async move {
            match parse_dsl(dsl_content).await {
                Ok(res) => set_parsed_data.set(Some(res)),
                Err(e) => set_error_msg.set(Some(e.to_string())),
            }
            set_is_parsing.set(false);
        });
    };

    #[cfg(not(feature = "ssr"))]
    let on_file_upload = move |ev: leptos::ev::Event| {
        use wasm_bindgen::JsCast;
        use web_sys::{FileReader, HtmlInputElement};

        let target = event_target::<HtmlInputElement>(&ev);
        if let Some(files) = target.files() {
            if let Some(file) = files.get(0) {
                let reader = FileReader::new().unwrap();
                let reader_clone = reader.clone();
                let onload = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                    if let Ok(res) = reader_clone.result() {
                        if let Some(text) = res.as_string() {
                            set_dsl.set(text);
                            set_error_msg.set(None);
                            set_parsed_data.set(None);
                        }
                    }
                })
                    as Box<dyn FnMut()>);

                reader.set_onload(Some(onload.as_ref().unchecked_ref()));
                onload.forget();
                let _ = reader.read_as_text(&file.into());
            }
        }
    };

    #[cfg(feature = "ssr")]
    let on_file_upload = move |_: leptos::ev::Event| {};

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            <div class="glass-panel" style="flex: 1; display: flex; flex-direction: column; overflow: hidden; padding: 1.5rem;">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem;">
                    <h2 style="margin: 0; font-size: 1.25rem;">"Schema Linter Playground"</h2>
                    <div style="display: flex; gap: 1rem; align-items: center;">
                        <input
                            type="file"
                            accept=".storm"
                            id="file-upload"
                            style="display: none;"
                            on:change=on_file_upload
                        />
                        <label
                            for="file-upload"
                            class="btn btn-secondary"
                            style="cursor: pointer; display: inline-block; font-weight: 500;"
                        >
                            "Upload .storm file"
                        </label>
                        <button
                            class="btn btn-primary"
                            disabled=move || is_parsing.get()
                            on:click=parse
                        >
                            {move || if is_parsing.get() { "Linting..." } else { "Lint" }}
                        </button>
                    </div>
                </div>

                <div style="flex: 1; display: flex; flex-direction: column; gap: 1rem; min-height: 0;">
                    <textarea
                        placeholder="Paste your .storm file contents here, or upload a file..."
                        style="padding: 1rem; border-radius: 6px; border: 1px solid var(--surface-border); background: var(--surface-elevated); color: var(--text-primary); font-family: monospace; flex: 1; resize: none; outline: none; font-size: 0.9rem; line-height: 1.5;"
                        class="custom-scrollbar"
                        prop:value=move || dsl.get()
                        on:input=move |ev| {
                            set_dsl.set(event_target_value(&ev));
                            set_error_msg.set(None);
                            set_parsed_data.set(None);
                        }
                    ></textarea>

                    <div style="min-height: 100px; max-height: 200px; overflow-y: auto; padding: 1rem; border-radius: 6px; border: 1px solid var(--surface-border); background: var(--surface-default);" class="custom-scrollbar">
                        {move || {
                            if let Some(msg) = error_msg.get() {
                                view! {
                                    <div style="color: var(--status-error); font-family: monospace; white-space: pre-wrap;">
                                        <strong>"Validation Error:"</strong>"\n"
                                        {msg}
                                    </div>
                                }.into_any()
                            } else if parsed_data.get().is_some() {
                                view! {
                                    <div style="color: var(--status-success); font-weight: 500; display: flex; align-items: center; gap: 0.5rem;">
                                        <span style="font-size: 1.25rem;">"✔"</span>
                                        "DSL is valid!"
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div style="color: var(--text-secondary); font-style: italic;">
                                        "Ready to lint."
                                    </div>
                                }.into_any()
                            }
                        }}
                    </div>
                </div>
            </div>
        </div>
    }
}
