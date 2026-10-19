//! The services wait for the database schema rather than migrating it themselves.

use sqlx::PgPool;
use std::time::Duration;
use stormchaser_core::db::migrations;

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is set by the test environment");
    PgPool::connect(&url).await.unwrap()
}

#[tokio::test]
async fn a_migrated_database_is_current_at_once() {
    let pool = pool().await;
    // Applying them again changes nothing
    migrations::run(&pool).await.unwrap();
    assert_eq!(
        migrations::applied_version(&pool).await.unwrap(),
        migrations::latest_version()
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        migrations::wait_until_current(&pool, Duration::ZERO),
    )
    .await
    .expect("no waiting")
    .unwrap();
}

#[tokio::test]
async fn a_database_behind_this_build_is_waited_for_and_then_refused() {
    let pool = pool().await;
    // A schema with nothing applied: a database of its own on the same server
    let schema = format!("migrations_test_{}", uuid::Uuid::new_v4().simple());
    // The name is ours, made from a UUID
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&pool)
        .await
        .unwrap();
    let url = std::env::var("DATABASE_URL").unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    let empty = sqlx::postgres::PgPoolOptions::new()
        .after_connect({
            let schema = schema.clone();
            move |conn, _| {
                let schema = schema.clone();
                Box::pin(async move {
                    sqlx::query(sqlx::AssertSqlSafe(format!("SET search_path TO {schema}")))
                        .execute(conn)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect_with(options)
        .await
        .unwrap();

    let err = migrations::wait_until_current(&empty, Duration::from_secs(3))
        .await
        .expect_err("an unmigrated schema is refused");
    assert!(
        err.to_string().contains("stormchaser-engine migrate"),
        "{err}"
    );

    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&pool)
        .await
        .unwrap();
}
