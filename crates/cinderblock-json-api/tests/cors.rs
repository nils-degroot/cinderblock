#![cfg(feature = "cors")]

use assert2::check;
use cinderblock_core::{
    Context, resource,
    serde::{Deserialize, Serialize},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

resource! {
    name = Test.Cors.Item;

    attributes {
        item_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }

        name String;
    }

    actions {
        read all;
        create add;
    }

    extensions {
        cinderblock_json_api {
            route = { method = GET; path = "/"; action = all; };
            route = { method = POST; path = "/"; action = add; };
        };
    }
}

fn build_app() -> axum::Router {
    cinderblock_json_api::RouterConfig::new(Context::new())
        .cors_permissive()
        .build()
}

#[tokio::test]
async fn permissive_cors_sets_allow_origin_header() {
    let app = build_app();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/test/cors/item/")
                .header("origin", "https://example.com")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.status() == axum::http::StatusCode::OK);
    check!(resp.headers().get("access-control-allow-origin").is_some());

    let allow_origin = resp.headers().get("access-control-allow-origin").unwrap();
    check!(allow_origin == "*");
}

#[tokio::test]
async fn preflight_request_returns_cors_headers() {
    let app = build_app();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("OPTIONS")
                .uri("/test/cors/item/")
                .header("origin", "https://example.com")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "content-type")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.headers().get("access-control-allow-origin").is_some());
    check!(resp.headers().get("access-control-allow-methods").is_some());
    check!(resp.headers().get("access-control-allow-headers").is_some());
}

#[tokio::test]
async fn no_cors_headers_without_origin() {
    let app = build_app();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/test/cors/item/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.status() == axum::http::StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    check!(json["data"].is_array());
}

#[tokio::test]
async fn custom_cors_layer_restricts_origin() {
    use cinderblock_json_api::tower_http::cors::CorsLayer;

    let app = cinderblock_json_api::RouterConfig::new(Context::new())
        .cors(
            CorsLayer::new()
                .allow_origin(
                    "https://allowed.example.com"
                        .parse::<axum::http::HeaderValue>()
                        .unwrap(),
                )
                .allow_methods([axum::http::Method::GET]),
        )
        .build();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/test/cors/item/")
                .header("origin", "https://allowed.example.com")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.status() == axum::http::StatusCode::OK);

    let allow_origin = resp.headers().get("access-control-allow-origin").unwrap();
    check!(allow_origin == "https://allowed.example.com");
}
