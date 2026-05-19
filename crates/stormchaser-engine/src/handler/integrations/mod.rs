mod approval;
mod email;
mod jinja;
mod lambda;
mod slack;
mod teams;
pub mod utils;
mod webhook;

pub use approval::handle_approval_notification;
pub use email::{handle_email_send, handle_test_report_email};
pub use jinja::handle_jinja_render;
pub use lambda::handle_lambda_invoke;
#[cfg(feature = "chatops-slack")]
pub use slack::handle_slack_message;
#[cfg(feature = "chatops-teams")]
pub use teams::handle_teams_message;
pub use webhook::handle_webhook_invoke;
