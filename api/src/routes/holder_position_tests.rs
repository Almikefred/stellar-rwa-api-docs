//! Tests for `holder_position::get`.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::holder_position::get;
use super::test_support::{asset, distribution, state_with};
use crate::indexer::Snapshot;
use crate::models::Holder;

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

/// Shared error body shape returned by the API.
#[derive(Debug, serde::Deserialize)]
struct ApiErrorBody {
    error: String,
    message: String,
}

async fn assert_client_error_with_shared_body(
    app: Router,
    uri: &str,
) -> ApiErrorBody {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router should not panic on malformed input");

    let status = response.status();
    assert!(
        status.is_client_error(),
        "expected a 4xx client error for {uri}, got {status}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);

    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body should be readable")
        .to_bytes();

    let body: ApiErrorBody = serde_json::from_slice(&bytes)
        .expect("error response should use the shared {error, message} body shape");
    assert!(!body.error.is_empty(), "error code should be present");
    assert!(!body.message.is_empty(), "error message should be present");
    body
}

#[tokio::test]
async fn malformed_query_parameters_return_client_error() {
    let app = super::holder_position::router(state_with(Snapshot::default()));

    // Non-numeric value for a numeric query parameter.
    assert_client_error_with_shared_body(
        app.clone(),
        "/holder_position/GA?limit=not-a-number",
    )
    .await;

    // Out-of-range value for a bounded query parameter.
    assert_client_error_with_shared_body(
        app.clone(),
        "/holder_position/GA?limit=999999999999999999999999",
    )
    .await;

    // Negative value where a non-negative integer is expected.
    assert_client_error_with_shared_body(
        app.clone(),
        "/holder_position/GA?offset=-1",
    )
    .await;

    // Malformed boolean-ish / enum-ish value.
    assert_client_error_with_shared_body(
        app,
        "/holder_position/GA?include_claimable=maybe",
    )
    .await;
}