//! One engine replica at a time holds the leader lock, and another takes it once the
//! holder's connection goes.

use sqlx::postgres::PgPoolOptions;
use std::env::var;
use stormchaser_engine::leader::try_lock;

async fn setup_db() -> sqlx::PgPool {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .unwrap()
}

#[tokio::test]
async fn one_replica_holds_the_lock_until_its_connection_goes() {
    // Two pools stand for two replicas. A key of this test's own, so that an engine
    // other tests start does not hold it.
    let first = setup_db().await;
    let second = setup_db().await;
    let key = uuid::Uuid::new_v4().as_u128() as i64;

    let held = try_lock(&first, key).await.unwrap();
    assert!(held.is_some(), "the first replica takes the free lock");
    assert!(
        try_lock(&second, key).await.unwrap().is_none(),
        "the second replica may not take it while the first holds it"
    );
    // A replica that took nothing gives its connection back, which sqlx does shortly
    // after it is dropped
    let mut returned = false;
    for _ in 0..50 {
        returned = second.num_idle() == second.size() as usize;
        if returned {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        returned,
        "the connection that took no lock went back to the pool"
    );

    drop(held);
    // Closing the connection is asynchronous; the server notices shortly
    let mut taken = None;
    for _ in 0..50 {
        taken = try_lock(&second, key).await.unwrap();
        if taken.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        taken.is_some(),
        "the second replica takes the lock once the first's connection is gone"
    );
}
