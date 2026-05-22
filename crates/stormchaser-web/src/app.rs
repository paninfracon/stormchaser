use crate::backends::BackendsList;
use crate::cron::CronList;
use crate::rules::RulesList;
use crate::runs::WorkflowRunsList;
use crate::webhooks::WebhooksList;

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Stylesheet, Title};
use leptos_router::{
    components::{Outlet, ParentRoute, Route, Router, Routes},
    path,
};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <Stylesheet id="leptos" href="/pkg/stormchaser-web.css"/>
                <link rel="icon" href="data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 100 100%22><text y=%22.9em%22 font-size=%2290%22>🌪️</text></svg>"/>
                <Title text="Stormchaser"/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn Layout() -> impl IntoView {
    let initial_is_dark = {
        #[cfg(not(feature = "ssr"))]
        {
            let window = leptos::prelude::window();
            if let Ok(Some(storage)) = window.local_storage() {
                if let Ok(Some(val)) = storage.get_item("theme") {
                    val == "dark"
                } else {
                    true
                }
            } else {
                true
            }
        }
        #[cfg(feature = "ssr")]
        {
            true
        }
    };

    let (is_dark, set_is_dark) = signal(initial_is_dark);

    let _theme_effect = Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            let window = leptos::prelude::window();
            let dark = is_dark.get();
            if let Some(document) = window.document() {
                if let Some(body) = document.body() {
                    let _ = body.set_attribute("data-theme", if dark { "dark" } else { "light" });
                }
            }
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item("theme", if dark { "dark" } else { "light" });
            }
        }
    });

    #[cfg(not(feature = "ssr"))]
    on_cleanup(move || {
        let _ = _theme_effect;
    });

    let toggle_theme = move |_| {
        set_is_dark.update(|d| *d = !*d);
    };

    view! {
        <div class="page-container fade-in" style="display: flex; flex-direction: column; height: 100vh;">
            <div style="display: flex; justify-content: space-between; align-items: center; padding-bottom: 1rem; border-bottom: 1px solid var(--surface-border); margin-bottom: 1.5rem;">
                <div style="display: flex; align-items: center; gap: 1rem;">
                    <div style="font-size: 2rem;">"🌪️"</div>
                    <h1 style="margin: 0; font-size: 1.5rem; font-weight: 700; background: linear-gradient(135deg, #3b82f6, #8b5cf6); -webkit-background-clip: text; -webkit-text-fill-color: transparent;">
                        "Stormchaser"
                    </h1>
                </div>
                <div style="display: flex; gap: 1rem; align-items: center;">
                    <a href="https://github.com/paninfracon/stormchaser" target="_blank" rel="noopener noreferrer" style="color: var(--text-secondary); text-decoration: none; display: flex; align-items: center; gap: 0.5rem; font-weight: 500;">
                        <svg height="24" viewBox="0 0 16 16" width="24" fill="currentColor">
                            <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"></path>
                        </svg>
                        "GitHub"
                    </a>
                    <button class="btn" style="background: transparent; color: var(--text-primary); border: 1px solid var(--surface-border);" on:click=toggle_theme>
                        {move || if is_dark.get() { "☀️ Light Mode" } else { "🌙 Dark Mode" }}
                    </button>
                    <Suspense fallback=|| view! { <a href="/auth/login" class="btn" rel="external">"Login with Dex"</a> }>
                        <a href="/auth/logout" class="btn" rel="external">
                            "Logout"
                        </a>
                    </Suspense>
                </div>
            </div>

            <div style="display: flex; gap: 1rem; margin-bottom: 1.5rem; border-bottom: 1px solid var(--surface-border); padding-bottom: 0.5rem;">
                <a href="/" class="tab-link">"Runs"</a>
                <a href="/backends" class="tab-link">"Backends"</a>
                <a href="/webhooks" class="tab-link">"Webhooks"</a>
                <a href="/rules" class="tab-link">"Rules"</a>
                <a href="/cron" class="tab-link">"Cron"</a>
            </div>

            <div style="flex: 1; display: flex; flex-direction: column; overflow: hidden;">
                <Outlet />
            </div>
        </div>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <ParentRoute path=path!("") view=Layout>
                        <Route path=path!("") view=WorkflowRunsList/>
                        <Route path=path!("backends") view=BackendsList/>
                        <Route path=path!("webhooks") view=WebhooksList/>
                        <Route path=path!("rules") view=RulesList/>
                        <Route path=path!("cron") view=CronList/>
                    </ParentRoute>
                </Routes>
            </main>
        </Router>
    }
}
