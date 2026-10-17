//! Tests for the `?fields=` middleware across list endpoints.

use axum::{body::Body, http::{Request, StatusCode}, middleware, routing::get, Router};
use tower::ServiceExt as _;

use super::field_select::field_select;
use super::test_support::{asset, state_with};
use crate::indexer::Snapshot;

fn app() -> Router {
    let state = state_with(Snapshot { assets: vec![asset(1), asset(2)], ..Default::default() });
    Router::new()
        .route("/assets", get(super::assets_query::list))
        .route("/assets/:id", get(super::assets::detail))
        .layer(middleware::from_fn(field_select))
        .with_state(state)
}

async fn call(uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn projects_list_and_detail() {
    let (status, body) = call("/assets?fields=id,symbol&limit=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([{"id": 1, "symbol": "TST"}]));
    let (_, body) = call("/assets/2?fields=name").await;
    assert_eq!(body, serde_json::json!({"name": "Test Asset"}));
}

#[tokio::test]
async fn unknown_field_is_a_clear_error() {
    let (status, body) = call("/assets?fields=id,nope").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_parameter");
    assert!(body["message"].as_str().unwrap().contains("nope"));
}

async fn call_raw(uri: &str) -> (StatusCode, Option<String>, serde_json::Value) {
    let response = app().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, content_type, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn malformed_query_parameters_are_clear_client_errors() {
    let cases = [
        "/assets?fields=id&limit=abc",
        "/assets?fields=id&limit=999999",
        "/assets?fields=id&offset=-1",
        "/assets?fields=id&active=maybe",
        "/assets?fields=id&sort=bogus",
        "/assets?fields=id&bogus=1",
    ];
    for uri in cases {
        let (status, content_type, body) = call_raw(uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "unexpected status for {uri}");
        assert!(
            content_type.as_deref().is_some_and(|ct| ct.starts_with("application/json")),
            "expected JSON content type for {uri}, got {content_type:?}"
        );
        let parsed: crate::error::ApiErrorBody =
            serde_json::from_value(body.clone()).unwrap_or_else(|e| panic!("bad error body for {uri}: {e}"));
        assert_eq!(parsed.error, "invalid_parameter", "unexpected error code for {uri}");
        assert!(!parsed.message.is_empty(), "empty message for {uri}");
        let obj = body.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["error", "message"], "unexpected body shape for {uri}");
    }
}
