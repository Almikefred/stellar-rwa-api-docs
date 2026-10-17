//! Tests for `assets_query::list`.

use std::collections::HashMap;

use axum::{extract::{Query, State}, http::StatusCode, middleware, response::IntoResponse, routing::get, Router};

use super::assets_query::list;
use super::field_select::field_select;
use super::test_support::{asset, state_with};
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
