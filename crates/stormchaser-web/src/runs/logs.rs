use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn StepLogsPanel(run_id: String, step_id: Signal<Option<String>>) -> impl IntoView {
    let run_id_clone = run_id.clone();
    #[cfg(not(feature = "ssr"))]
    let run_id_for_sse = run_id.clone();

    let logs_signal = RwSignal::new(None::<Vec<String>>);

    let _fetch_effect = Effect::new(move |_| {
        if let Some(sid) = step_id.get() {
            let rid = run_id_clone.clone();
            spawn_local(async move {
                logs_signal.set(None);
                if let Ok(initial_logs) = crate::api::fetch_step_logs(rid, sid, Some(100)).await {
                    logs_signal.set(Some(initial_logs));
                } else {
                    logs_signal.set(Some(Vec::new()));
                }
            });
        } else {
            logs_signal.set(Some(Vec::new()));
        }
    });

    #[cfg(not(feature = "ssr"))]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;
        use web_sys::{EventSource, MessageEvent};

        let _logs_sse_effect = Effect::new(move |_| {
            if let Some(sid) = step_id.get() {
                let rid = run_id_for_sse.clone();
                if let Ok(es) =
                    EventSource::new(&format!("/api/v1/runs/{}/steps/{}/logs/stream", rid, sid))
                {
                    let es_clone = es.clone();
                    let on_log = Closure::wrap(Box::new(move |event: MessageEvent| {
                        if let Some(data) = event.data().as_string() {
                            logs_signal.update(|v| {
                                if let Some(list) = v {
                                    list.push(data);
                                } else {
                                    *v = Some(vec![data]);
                                }
                            });

                            // Force a scroll to bottom
                            if let Some(window) = web_sys::window() {
                                if let Some(doc) = window.document() {
                                    if let Some(el) = doc.get_element_by_id("logs-container") {
                                        el.set_scroll_top(el.scroll_height());
                                    }
                                }
                            }
                        }
                    })
                        as Box<dyn FnMut(MessageEvent)>);

                    es.add_event_listener_with_callback("log", on_log.as_ref().unchecked_ref())
                        .unwrap();

                    let ptr = Box::into_raw(Box::new((es_clone, on_log))) as usize;
                    on_cleanup(move || {
                        let raw = ptr as *mut (EventSource, Closure<dyn FnMut(MessageEvent)>);
                        let boxed = unsafe { Box::from_raw(raw) };
                        boxed.0.close();
                    });
                }
            }
        });

        on_cleanup(move || {
            let _ = _logs_sse_effect;
        });
    }

    view! {
        <div style="flex: 1; display: flex; flex-direction: column; background: #1e1e1e; color: #d4d4d4; font-family: monospace; overflow: hidden; height: 100%;">
            <div style="padding: 0.5rem 1rem; background: #2d2d2d; border-bottom: 1px solid #404040; font-size: 0.85rem; display: flex; justify-content: space-between;">
                <span>"Logs"</span>
            </div>
            <div id="logs-container" style="flex: 1; overflow-y: auto; padding: 1rem; font-size: 0.85rem; line-height: 1.5; white-space: pre-wrap; word-break: break-all;">
                {move || {
                    let sid_opt = step_id.get();
                    if sid_opt.is_none() {
                        return view! { <div style="color: #808080;">"Select a step to view logs"</div> }.into_any();
                    }

                    let logs_opt = logs_signal.get();

                    match logs_opt {
                        None => view! { <div style="color: #808080;">"Loading logs..."</div> }.into_any(),
                        Some(logs) if logs.is_empty() => view! { <div style="color: #808080;">"No logs available for this step."</div> }.into_any(),
                        Some(logs) => {
                            let content = logs.join("\n");
                            view! { <div>{content}</div> }.into_any()
                        }
                    }
                }}
            </div>
        </div>
    }
}
