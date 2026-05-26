use leptos::prelude::*;

#[component]
pub fn GrafanaView() -> impl IntoView {
    let grafana_res = Resource::new(
        || (),
        |_| async move { crate::api::get_grafana_url().await.unwrap_or(None) },
    );

    view! {
        <Suspense fallback=|| view! { <div class="glass-panel" style="flex: 1; display: flex; justify-content: center; align-items: center;">"Loading Grafana..."</div> }>
            {move || {
                if let Some(Some(url)) = grafana_res.get() {
                    let embed_url = format!("{}/d/stormchaser-perf/stormchaser?orgId=1&kiosk", url);
                    view! {
                        <div class="glass-panel" style="flex: 1; display: flex; flex-direction: column; padding: 0; overflow: hidden; margin: 0;">
                            <div style="display: flex; justify-content: flex-end; padding: 0.75rem 1rem; border-bottom: 1px solid var(--surface-border); background: rgba(0,0,0,0.02);">
                                <a href=url target="_blank" class="btn" style="background: var(--surface-light); border: 1px solid var(--surface-border); box-shadow: 0 2px 4px rgba(0,0,0,0.05); display: flex; align-items: center; gap: 0.5rem; text-decoration: none; font-size: 0.85rem; color: var(--text-primary); padding: 0.4rem 0.8rem; border-radius: 6px;">
                                    "↗ Full Grafana UI"
                                </a>
                            </div>
                            <iframe src=embed_url style="flex: 1; border: none; width: 100%; height: 100%;"></iframe>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="glass-panel" style="flex: 1; display: flex; justify-content: center; align-items: center; color: var(--text-secondary);">
                            "Grafana URL not configured."
                        </div>
                    }.into_any()
                }
            }}
        </Suspense>
    }
}
