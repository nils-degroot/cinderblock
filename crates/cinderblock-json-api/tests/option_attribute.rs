use assert2::check;
use cinderblock_core::{
    Context, resource,
    serde::{Deserialize, Serialize},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    cinderblock_json_api::utoipa::ToSchema,
)]
enum Priority {
    #[default]
    Low,
    Medium,
    High,
}

cinderblock_json_api::impl_field_schema!(Priority);

resource! {
    name = Test.JsonApi.OptionItem;

    attributes {
        item_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }

        title String;
        description Option<String>;
        priority Option<Priority>;
    }

    actions {
        read all;

        create add;

        update edit;
    }

    extensions {
        cinderblock_json_api {
            route = { method = GET; path = "/"; action = all; };
            route = { method = POST; path = "/"; action = add; };
            route = { method = PATCH; path = "/{primary_key}"; action = edit; };
        };
    }
}

fn build_app() -> axum::Router {
    cinderblock_json_api::router(Context::new())
}

#[tokio::test]
async fn create_with_optional_fields_set_to_null() {
    let app = build_app();

    let body = serde_json::json!({
        "title": "No description",
        "description": null,
        "priority": null,
    });

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/test/json-api/option-item/")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.status() == axum::http::StatusCode::CREATED);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    check!(json["data"]["title"] == "No description");
    check!(json["data"]["description"] == serde_json::Value::Null);
    check!(json["data"]["priority"] == serde_json::Value::Null);
}

#[tokio::test]
async fn create_with_optional_fields_set_to_values() {
    let app = build_app();

    let body = serde_json::json!({
        "title": "Has everything",
        "description": "A real description",
        "priority": "High",
    });

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/test/json-api/option-item/")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.status() == axum::http::StatusCode::CREATED);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    check!(json["data"]["title"] == "Has everything");
    check!(json["data"]["description"] == "A real description");
    check!(json["data"]["priority"] == "High");
}

#[tokio::test]
async fn create_with_optional_fields_omitted() {
    let app = build_app();

    let body = serde_json::json!({
        "title": "Minimal",
    });

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/test/json-api/option-item/")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.status() == axum::http::StatusCode::CREATED);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    check!(json["data"]["title"] == "Minimal");
    check!(json["data"]["description"] == serde_json::Value::Null);
    check!(json["data"]["priority"] == serde_json::Value::Null);
}

#[tokio::test]
async fn read_returns_optional_fields() {
    let ctx = Context::new();

    cinderblock_core::create::<OptionItem, Add>(
        AddInput {
            title: "With desc".into(),
            description: Some("desc value".into()),
            priority: Some(Priority::High),
        },
        &ctx,
    )
    .await
    .expect("seed item with values");

    cinderblock_core::create::<OptionItem, Add>(
        AddInput {
            title: "Without desc".into(),
            description: None,
            priority: None,
        },
        &ctx,
    )
    .await
    .expect("seed item without values");

    let app = build_app();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/test/json-api/option-item/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    check!(resp.status() == axum::http::StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let data = json["data"].as_array().expect("data should be an array");
    check!(data.len() >= 2);

    let with_desc = data
        .iter()
        .find(|item| item["title"] == "With desc")
        .unwrap();
    check!(with_desc["description"] == "desc value");
    check!(with_desc["priority"] == "High");

    let without_desc = data
        .iter()
        .find(|item| item["title"] == "Without desc")
        .unwrap();
    check!(without_desc["description"] == serde_json::Value::Null);
    check!(without_desc["priority"] == serde_json::Value::Null);
}

#[tokio::test]
async fn openapi_schema_marks_optional_fields_as_not_required() {
    let mut merged = cinderblock_json_api::utoipa::openapi::OpenApiBuilder::new()
        .info(
            cinderblock_json_api::utoipa::openapi::InfoBuilder::new()
                .title("test")
                .version("0.0.0")
                .build(),
        )
        .build();

    for endpoint in inventory::iter::<cinderblock_json_api::ResourceEndpoint> {
        if let Some(openapi_fn) = endpoint.openapi {
            merged.merge(openapi_fn());
        }
    }

    let spec = serde_json::to_value(&merged).unwrap();

    let option_item_schema = &spec["components"]["schemas"]["OptionItem"];
    let required = option_item_schema["required"]
        .as_array()
        .expect("required should be an array");

    let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    check!(required_names.contains(&"item_id"));
    check!(required_names.contains(&"title"));
    check!(!required_names.contains(&"description"));
    check!(!required_names.contains(&"priority"));
}
