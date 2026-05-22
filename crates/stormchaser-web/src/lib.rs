#![recursion_limit = "512"]

pub mod api;
pub mod app;
pub mod backends;
pub mod cron;
pub mod models;
pub mod rules;
pub mod runs;
pub mod webhooks;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
