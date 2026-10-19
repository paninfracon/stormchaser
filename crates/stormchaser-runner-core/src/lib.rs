//! What the Docker and Kubernetes runners share.

/// Encryption of the step definitions runners keep on their containers and Jobs.
pub mod crypto;
/// Reading the agent's marked log lines.
pub mod markers;
/// A runner's life around the [`runner::Backend`] each kind of runner implements.
pub mod runner;

pub use runner::{run, Backend, RunnerContext, RunnerSettings};
