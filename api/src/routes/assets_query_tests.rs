//! Tests for `assets_query::list`.

use std::collections::HashMap;

use axum::{extract::{Query, State}, http::StatusCode, middleware, response::IntoResponse, routing::get, Router};

use super::assets_query::list;
use super::field_select::field_select;
use super::test_support::{asset, state_with};
use crate::error::ApiErrorBody;
use crate::indexer::Snapshot;

fn params(pairs: &[(&str, &str)]) -> Query<HashMap<String, String>> {
    Query(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

fn state() -> crate::indexer::AppState {
    let mut a = asset(1);
    a.valuation_usd = 10.0;
    let mut b = asset(2);
    b.valuation_usd = 30.0;
    b.asset_type = "bond".to_string();
    let mut c = asset(3);
    c.valuation_usd = 20.0;
    state_with(Snapshot { assets: vec![a, b, c], ..Default::default() })
}

async fn ids(query: &[(&str, &str)]) -> Vec<u64> {
    let response = list(State(state()), params(query)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    v.as_array().unwrap().iter().map(|a| a["id"].as_u64().unwrap()).collect()
}

#[tokio::test]
async fn sorts_by_valuation_and_filters_by_type() {
    assert_eq!(ids(&[("sort", "valuation")]).await, vec![2, 3, 1]);
    assert_eq!(ids(&[("sort", "valuation"), ("order", "asc")]).await, vec![1, 3, 2]);
    assert_eq!(ids(&[("asset_type", "bond")]).await, vec![2]);
}

#[tokio::test]
async fn field_selection_applies_to_list() {
    let app = Router::new()
        .route("/assets", get(list))
        .layer(middleware::from_fn(field_select))
        .with_state(state());

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/assets?fields=id,symbol")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    for item in arr {
        let obj = item.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["id", "symbol"]);
    }

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/assets?fields=id,nope")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["error"], "invalid_parameter");
    assert!(v["message"].as_str().unwrap().contains("nope"));
}

#[tokio::test]
async fn invalid_parameters_are_rejected() {
    for q in [
        vec![("sort", "name")],
        vec![("order", "up"), ("sort", "valuation")],
        vec![("order", "asc")],
        vec![("active", "maybe")],
        vec![("limit", "x")],
        vec![("bogus", "1")],
    ] {
        let response = list(State(state()), params(&q)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{q:?}");
    }
}

#[tokio::test]
async fn malformed_and_out_of_range_parameters_return_shared_error_body() {
    for q in [
        vec![("limit", "abc")],
        vec![("offset", "-1")],
        vec![("limit", "999999")],
        vec![("active", "maybe")],
        vec![("sort", "name")],
        vec![("order", "up")],
        vec![("bogus", "1")],
    ] {
        let response = list(State(state()), params(&q)).await.into_response();
        let status = response.status();
        assert_eq!(status, StatusCode::BAD_REQUEST, "{q:?}");
        assert!(!status.is_server_error(), "{q:?}");

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let obj = v.as_object().expect("error body must be a JSON object");
        assert_eq!(obj.len(), 2, "{q:?}: {v}");
        assert_eq!(obj.get("error").and_then(|e| e.as_str()), Some("invalid_parameter"), "{q:?}");

        let parsed: ApiErrorBody = serde_json::from_slice(&body).unwrap();
        assert!(!parsed.message.is_empty(), "{q:?}");
    }
}

#[tokio::test]
async fn empty_result_is_well_formed_empty_array() {
    let response = list(State(state()), params(&[("asset_type", "does-not-exist")])).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = v.as_array().expect("empty result must be a JSON array, not null or an error");
    assert!(arr.is_empty());
}

#[tokio::test]
async fn empty_result_is_well_formed_array() {
    // No asset matches this filter, so the route must return a well-formed
    // empty JSON array (not `null`, not an error).
    let response = list(State(state()), params(&[("asset_type", "equity")])).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = v.as_array().expect("empty result must be a JSON array, not null");
    assert!(arr.is_empty(), "expected empty array, got {arr:?}");
}

#[tokio::test]
async fn empty_snapshot_returns_empty_array() {
    // With no assets indexed at all, the route still returns `[]`.
    let empty_state = state_with(Snapshot::default());
    let response = list(State(empty_state), params(&[])).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = v.as_array().expect("empty result must be a JSON array, not null");
    assert!(arr.is_empty(), "expected empty array, got {arr:?}");
}
