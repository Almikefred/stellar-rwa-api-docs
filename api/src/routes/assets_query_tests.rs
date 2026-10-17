//! Tests for `assets_query::list`.

use std::collections::HashMap;

use axum::{
    body::Body,
    extract::{ConnectInfo, Query, State},
    http::{header, Request, StatusCode},
    response::IntoResponse,
};
use tower::ServiceExt;

use super::assets_query::list;
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

fn get(uri: &str, if_none_match: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri(uri);
    if let Some(tag) = if_none_match {
        builder = builder.header(header::IF_NONE_MATCH, tag);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 0))));
    request
}

#[tokio::test]
async fn sets_etag_and_honours_if_none_match() {
    let app = super::router(state());

    let response = app.clone().oneshot(get("/v1/assets", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("ETag header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        etag.starts_with("ledger-") && etag.ends_with('"'),
        "unexpected ETag format: {etag}"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!body.is_empty());

    let response = app
        .clone()
        .oneshot(get("/v1/assets", Some(&etag)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(body.is_empty());

    let response = app
        .oneshot(get("/v1/assets", Some("\"ledger-stale\"")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::ETAG).unwrap().to_str().unwrap(),
        etag
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!body.is_empty());
}
