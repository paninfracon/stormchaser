use super::create::CreateBackendModal;
use super::table::ConnectionsTable;
use crate::api::fetch_connections;
use leptos::prelude::*;
#[component]
pub fn BackendsList() -> impl IntoView {
    let connections_resource = Resource::new(|| (), |_| async { fetch_connections().await });
    let (show_create_modal, set_show_create_modal) = signal(false);

    view! {
        <div style="display: flex; gap: 1rem; flex: 1; min-height: 0;">
            {move || if show_create_modal.get() {
                view! {
                    <CreateBackendModal
                        on_close=Callback::new(move |_| set_show_create_modal.set(false))
                        on_success=Callback::new(move |_| {
                            set_show_create_modal.set(false);
                            connections_resource.refetch();
                        })
                    />
                }.into_any()
            } else { ().into_any() }}
            <div class="glass-panel table-container" style="flex: 1; overflow: auto;">
                <div style="padding: 1.5rem; display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid var(--surface-border);">
                    <h2 style="margin: 0; font-size: 1.25rem; font-weight: 600;">"Backend Connections"</h2>
                    <button class="btn-primary" on:click=move |_| set_show_create_modal.set(true)>
                        "Create Connection"
                    </button>
                </div>

                <Suspense fallback=|| view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading connections..."</div> }>
                    {move || match connections_resource.get() {
                        Some(Ok(connections)) => view! {
                            <ConnectionsTable connections=connections on_delete_success=Callback::new(move |_| connections_resource.refetch()) />
                        }.into_any(),
                        Some(Err(e)) => view! {
                            <div style="padding: 2rem; text-align: center; color: var(--status-error);">
                                "Error loading connections: " {e.to_string()}
                            </div>
                        }.into_any(),
                        None => view! { <div style="padding: 2rem; text-align: center; color: var(--text-secondary);">"Loading connections..."</div> }.into_any(),
                    }}
                </Suspense>
            </div>
        </div>
    }
}
