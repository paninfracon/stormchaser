//! One engine replica at a time holds the leader lock, and another takes it once the
//! holder's connection goes.

mod common;

use common::setup_db;
use stormchaser_engine::leader::try_lock;

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

#[tokio::test]
async fn the_server_ends_a_lock_holders_session_once_it_goes_quiet() {
    // Its host can die without closing the connection; the server must not keep the
    // lock for as long as TCP would
    let pool = setup_db().await;
    let key = uuid::Uuid::new_v4().as_u128() as i64;
    let mut held = try_lock(&pool, key)
        .await
        .unwrap()
        .expect("the lock is free");

    for (setting, value) in [
        ("SHOW idle_session_timeout", "30s"),
        ("SHOW tcp_keepalives_idle", "10"),
        ("SHOW tcp_keepalives_interval", "5"),
        ("SHOW tcp_keepalives_count", "3"),
    ] {
        let shown: String = sqlx::query_scalar(setting)
            .fetch_one(&mut held)
            .await
            .unwrap();
        // The server reports keepalive settings as 0 on a Unix socket, where they do
        // not apply
        assert!(
            shown == value || (setting.contains("tcp_") && shown == "0"),
            "{setting} is {shown}"
        );
    }
}
