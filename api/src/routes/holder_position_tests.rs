//! Tests for `holder_position::get`.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

use super::holder_position::get;
use super::test_support::{asset, distribution, state_with};
use crate::indexer::Snapshot;
use crate::models::Holder;
use crate::routes::router;

#[tokio::test]
async fn aggregates_positions_and_claimable() {
    let mut a = asset(1);
    a.total_supply = "1000".to_string();
    a.valuation_usd = 200.0;
    let mut d = distribution(7, 5);
    d.total_amount = "1000".to_string();
    d.distributed = "200".to_string();
    let mut snapshot = Snapshot { assets: vec![a], ..Default::default() };
    snapshot.holders.insert(
        1,
        vec![Holder { address: "GA".into(), balance: "250".into(), share_percent: 25.0 }],
    );
    snapshot.dividends.insert(1, vec![d]);

    let position = get(State(state_with(snapshot)), Path("GA".into())).await.0;
    assert_eq!(position.asset_count, 1);
    assert_eq!(position.total_value_usd, 50.0);
    assert_eq!(position.positions[0].claimable[0].amount, "200");
}

#[tokio::test]
async fn unknown_address_has_empty_position() {
    let position = get(State(state_with(Snapshot::default())), Path("GX".into())).await.0;
    assert_eq!(position.asset_count, 0);
    assert!(position.positions.is_empty());
}

#[tokio::test]
async fn sets_etag_and_honours_if_none_match() {
    let mut snapshot = Snapshot::default();
    snapshot.last_indexed_ledger = 42;

    let app = router(state_with(snapshot));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/holders/GA/position")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("ETag header should be set")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(etag, "\"ledger-42\"");

    let cache_control = response
        .headers()
        .get(header::CACHE_CONTROL)
        .expect("Cache-Control header should be set")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(
        cache_control,
        format!("public, max-age={}", crate::routes::POLL_INTERVAL)
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/holders/GA/position")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.is_empty());
}