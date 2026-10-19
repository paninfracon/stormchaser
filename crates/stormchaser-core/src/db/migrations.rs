//! The database schema: applying migrations, and waiting for someone else to.
//!
//! One thing applies migrations: `stormchaser-engine migrate`, run by a Helm hook Job
//! or a one-shot compose service before the services start. The API and the engine
//! wait at start-up until the schema is at least as new as the migrations they were
//! built with, so that they never run against tables they do not know.

use anyhow::{bail, Context, Result};
use sqlx::migrate::Migrator;
use sqlx::PgPool;
use std::time::{Duration, Instant};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Makes a service apply the migrations itself at start-up, as every service did
/// before there was a migration step. For running a service on its own in development.
pub const MIGRATE_ON_START_ENV: &str = "STORMCHASER_MIGRATE_ON_START";

/// How long a service waits for the schema to be migrated before giving up, as a
/// duration such as `5m`.
pub const SCHEMA_WAIT_ENV: &str = "STORMCHASER_SCHEMA_WAIT";

const DEFAULT_SCHEMA_WAIT: Duration = Duration::from_secs(5 * 60);
const POLL: Duration = Duration::from_secs(2);
const REPORT_EVERY: Duration = Duration::from_secs(30);

/// Applies every migration the database does not have yet.
pub async fn run(pool: &PgPool) -> Result<()> {
    tracing::info!("Running database migrations...");
    MIGRATOR
        .run(pool)
        .await
        .context("Failed to apply database migrations")?;
    tracing::info!("Database migrations completed successfully");
    Ok(())
}

/// The newest migration this build knows.
pub fn latest_version() -> i64 {
    MIGRATOR
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
        .map(|m| m.version)
        .max()
        .unwrap_or(0)
}

/// The newest migration applied to the database, or 0 when none has been.
pub async fn applied_version(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
        .fetch_one(pool)
        .await?;
    if !exists {
        return Ok(0);
    }
    sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success")
        .fetch_one(pool)
        .await
}

/// Gets the schema ready for this service: applies the migrations when
/// `STORMCHASER_MIGRATE_ON_START=true`, and otherwise waits, for up to
/// `STORMCHASER_SCHEMA_WAIT` (5 minutes by default), until they have been applied.
pub async fn prepare(pool: &PgPool) -> Result<()> {
    let migrate_on_start = match std::env::var(MIGRATE_ON_START_ENV) {
        Ok(value) => stormchaser_tls::parse_switch(MIGRATE_ON_START_ENV, &value)?,
        Err(_) => false,
    };
    if migrate_on_start {
        return run(pool).await;
    }
    let wait = match std::env::var(SCHEMA_WAIT_ENV) {
        Ok(value) => humantime::parse_duration(value.trim())
            .with_context(|| format!("{} is not a duration: {}", SCHEMA_WAIT_ENV, value))?,
        Err(_) => DEFAULT_SCHEMA_WAIT,
    };
    wait_until_current(pool, wait).await
}

/// Waits until the database has every migration this build knows, for up to `wait`.
pub async fn wait_until_current(pool: &PgPool, wait: Duration) -> Result<()> {
    let wanted = latest_version();
    let started = Instant::now();
    let mut reported = None::<Instant>;
    loop {
        match applied_version(pool).await {
            Ok(applied) if applied >= wanted => {
                if applied > wanted {
                    tracing::warn!(
                        "The database schema ({}) is newer than this build's ({}); a newer \
                         version is being rolled out",
                        applied,
                        wanted
                    );
                }
                return Ok(());
            }
            Ok(applied) => {
                if reported.is_none_or(|at| at.elapsed() >= REPORT_EVERY) {
                    tracing::info!(
                        "Waiting for the database schema to be migrated to {} (it is at {}); \
                         run `stormchaser-engine migrate`, or set {}=true",
                        wanted,
                        applied,
                        MIGRATE_ON_START_ENV
                    );
                    reported = Some(Instant::now());
                }
            }
            Err(e) => tracing::warn!("Could not read the database schema version: {}", e),
        }
        if started.elapsed() >= wait {
            bail!(
                "The database schema was not migrated to {} within {:?}. Run \
                 `stormchaser-engine migrate`, or set {}=true",
                wanted,
                wait,
                MIGRATE_ON_START_ENV
            );
        }
        tokio::time::sleep(POLL).await;
    }
}
