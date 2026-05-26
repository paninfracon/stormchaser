#![recursion_limit = "512"]

pub mod api;
pub mod app;
pub mod approvals;
pub mod backends;
pub mod cron;
pub mod grafana;
pub mod models;
pub mod rules;
pub mod run_modal;
pub mod runs;
pub mod schema_lint;
pub mod webhooks;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
