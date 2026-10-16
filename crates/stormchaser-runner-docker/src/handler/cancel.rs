use crate::container_machine::cancel::stop_run_containers;
use bollard::Docker;
use stormchaser_model::run_cancel;
use tracing::{info, warn};

/// Acts on a cancellation from the engine: stops the run's containers now, and looks
/// again shortly for any that were still being created.
pub async fn handle_cancel(message: async_nats::jetstream::Message, docker: Docker) {
    let Some(event) = run_cancel::parse(&message) else {
        warn!("Ignoring a message on the cancel subject that is not a cancellation");
        return;
    };
    let run_id = event.run_id.into_inner();
    run_cancel::mark_cancelled(run_id);

    let stopped = stop_run_containers(&docker, run_id).await;
    if stopped > 0 {
        info!(
            "Run {} was cancelled ({}): stopped {} container(s)",
            run_id, event.reason, stopped
        );
    }
    for delay in run_cancel::RESWEEP_AFTER {
        tokio::time::sleep(delay).await;
        let late = stop_run_containers(&docker, run_id).await;
        if late > 0 {
            info!(
                "Run {} was cancelled: stopped {} container(s) that started late",
                run_id, late
            );
        }
    }
}
