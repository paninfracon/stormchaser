//! Asking policy whether a run may use a connection.
//!
//! Connections hold credentials, and a workflow refers to one by name. Before the engine
//! uses a connection on a run's behalf it asks policy, as the user the run acts for
//! (whoever started it, or whoever created the schedule or rule that did). Nothing is
//! enforced only in a debug build started with `STORMCHASER_AUTHZ_DISABLED=true`.

use anyhow::{bail, Context, Result};
use sqlx::PgPool;
use std::sync::{Arc, OnceLock};
use stormchaser_model::auth::{OpaClient, OpaResource, OpaSubject, ResourceOpaContext};
use stormchaser_model::{Connection, RunId};

static POLICY: OnceLock<Arc<OpaClient>> = OnceLock::new();

/// Gives this module the engine's policy client. Called once at start-up.
pub fn install(policy: Arc<OpaClient>) {
    let _ = POLICY.set(policy);
}

fn policy() -> Option<&'static Arc<OpaClient>> {
    POLICY.get().filter(|client| client.is_configured())
}

/// Fails unless `user` may use `connection`.
pub async fn ensure_user_may_use(pool: &PgPool, user: &str, connection: &Connection) -> Result<()> {
    let Some(policy) = policy() else {
        return Ok(());
    };

    let owner: Option<String> =
        sqlx::query_scalar("SELECT created_by FROM connections WHERE id = $1")
            .bind(connection.id)
            .fetch_optional(pool)
            .await?
            .flatten();

    let allowed = policy
        .check_context(ResourceOpaContext {
            action: "use",
            resource: OpaResource {
                kind: "connection",
                id: &connection.name,
                owner: owner.as_deref(),
            },
            // The engine knows the user only by the value the run recorded
            subject: OpaSubject {
                sub: user,
                email: Some(user),
                groups: &[],
            },
        })
        .await
        .context("Policy check for connection use failed")?;

    if !allowed {
        bail!(
            "Policy does not allow '{}' to use connection '{}'",
            user,
            connection.name
        );
    }
    Ok(())
}

/// Fails unless the user `run_id` acts for may use `connection`.
pub async fn ensure_run_may_use(
    pool: &PgPool,
    run_id: RunId,
    connection: &Connection,
) -> Result<()> {
    if policy().is_none() {
        return Ok(());
    }
    let user: String =
        sqlx::query_scalar("SELECT initiating_user FROM workflow_runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(pool)
            .await?
            .with_context(|| format!("Run {} not found while checking connection use", run_id))?;
    ensure_user_may_use(pool, &user, connection).await
}
