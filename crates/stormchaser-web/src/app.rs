use crate::approvals::ApprovalsList;
use crate::backends::BackendsList;
use crate::cron::CronList;
use crate::rules::RulesList;
use crate::runs::WorkflowRunsList;
use crate::schema_lint::SchemaLinter;
use crate::webhooks::WebhooksList;

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Stylesheet, Title};
use leptos_router::{
    components::{Outlet, ParentRoute, Route, Router, Routes},
    path,
};

/// The Content-Security-Policy for a rendered page.
///
/// Scripts run only from this origin or with this response's nonce, which Leptos puts on
/// the inline scripts it generates for hydration and streaming. Styles still allow inline
/// `style` attributes, which the views use throughout; the stylesheet imports its font
/// from Google Fonts. The only thing that may be framed is the configured Grafana.
/// `dev_reload` adds the live-reload socket used under `cargo leptos watch`.
#[cfg(feature = "ssr")]
pub fn content_security_policy(nonce: &str, grafana: Option<&str>, dev_reload: bool) -> String {
    let frames = grafana.unwrap_or("'none'");
    let connect = if dev_reload {
        "'self' ws://127.0.0.1:* ws://localhost:*"
    } else {
        "'self'"
    };
    format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}' 'wasm-unsafe-eval'; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
         font-src 'self' https://fonts.gstatic.com; \
         img-src 'self' data:; \
         connect-src {connect}; \
         frame-src {frames}; \
         object-src 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         frame-ancestors 'none'"
    )
}

pub fn shell(options: LeptosOptions) -> impl IntoView {
    // The policy has to name the nonce of this very response, so it is set here, where
    // the nonce is known, and not by the server's blanket header layer.
    #[cfg(feature = "ssr")]
    if let (Some(nonce), Some(response)) = (
        leptos::nonce::use_nonce(),
        use_context::<leptos_axum::ResponseOptions>(),
    ) {
        let policy = content_security_policy(
            &nonce,
            crate::api::grafana_origin().as_deref(),
            std::env::var("LEPTOS_WATCH").is_ok(),
        );
        if let Ok(value) = axum::http::HeaderValue::from_str(&policy) {
            response.insert_header(axum::http::header::CONTENT_SECURITY_POLICY, value);
        }
    }

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
                    {
                        let auth_res = Resource::new(|| (), |_| async move {
                            crate::api::check_auth().await.unwrap_or(false)
                        });
                        view! {
                            <Suspense fallback=|| view! { <span>"..."</span> }>
                                {move || {
                                    if auth_res.get().unwrap_or(false) {
                                        view! {
                                            // A form, not a link: logging out changes state,
                                            // and a GET could be triggered by any page
                                            <form method="post" action="/auth/logout" style="display: inline; margin: 0;">
                                                <button type="submit" class="btn">"Logout"</button>
                                            </form>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <a href="/auth/login" class="btn" rel="external">
                                                "Login with Dex"
                                            </a>
                                        }.into_any()
                                    }
                                }}
                            </Suspense>
                        }
                    }
                </div>
            </div>

            <div style="display: flex; gap: 1rem; margin-bottom: 1.5rem; border-bottom: 1px solid var(--surface-border); padding-bottom: 0.5rem;">
                <a href="/" class="tab-link">"Runs"</a>
                <a href="/approvals" class="tab-link">"Approvals"</a>
                <a href="/backends" class="tab-link">"Backends"</a>
                <a href="/webhooks" class="tab-link">"Webhooks"</a>
                <a href="/rules" class="tab-link">"Rules"</a>
                <a href="/cron" class="tab-link">"Cron"</a>
                <a href="/schema" class="tab-link">"Schema/Lint"</a>
                {
                    let grafana_res = Resource::new(|| (), |_| async move {
                        crate::api::get_grafana_url().await.unwrap_or(None)
                    });
                    view! {
                        <Suspense fallback=|| view! { <span></span> }>
                            {move || {
                                if let Some(Some(_url)) = grafana_res.get() {
                                    view! {
                                        <a href="/grafana" class="tab-link">"Grafana"</a>
                                    }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                        </Suspense>
                    }
                }
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
                        <Route path=path!("approvals") view=ApprovalsList/>
                        <Route path=path!("backends") view=BackendsList/>
                        <Route path=path!("webhooks") view=WebhooksList/>
                        <Route path=path!("rules") view=RulesList/>
                        <Route path=path!("cron") view=CronList/>
                        <Route path=path!("schema") view=SchemaLinter/>
                        <Route path=path!("grafana") view=crate::grafana::GrafanaView/>
                    </ParentRoute>
                </Routes>
            </main>
        </Router>
    }
}

#[cfg(all(test, feature = "ssr"))]
mod csp_tests {
    use super::content_security_policy;

    #[test]
    fn scripts_need_this_response_nonce_and_nothing_may_frame_the_page() {
        let policy = content_security_policy("abc123", None, false);
        assert!(policy.contains("script-src 'self' 'nonce-abc123' 'wasm-unsafe-eval';"));
        assert!(!policy.contains("script-src 'self' 'unsafe-inline'"));
        assert!(policy.contains("frame-ancestors 'none'"));
        assert!(policy.contains("frame-src 'none';"));
        assert!(policy.contains("object-src 'none';"));
        assert!(policy.contains("connect-src 'self';"));
        // One line: a header value cannot hold a line break
        assert!(!policy.contains('\n'));
    }

    #[test]
    fn only_the_configured_grafana_may_be_framed() {
        let policy = content_security_policy("n", Some("https://grafana.example.com"), false);
        assert!(policy.contains("frame-src https://grafana.example.com;"));
    }

    #[test]
    fn the_reload_socket_is_allowed_only_in_watch_mode() {
        assert!(content_security_policy("n", None, true).contains("ws://127.0.0.1:*"));
        assert!(!content_security_policy("n", None, false).contains("ws:"));
    }
}
