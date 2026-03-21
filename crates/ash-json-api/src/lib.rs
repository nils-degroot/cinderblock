// # Ash JSON API Extension
//
// Provides a JSON REST API extension for Ash resources. When a resource
// declares `ash_json_api` in its `extensions { ... }` block, the `resource!`
// macro generates route registration code that automatically registers
// endpoints via `inventory`.
//
// Usage in application code:
//
// ```rust
// let ctx = ash_core::Context::new("my_app").await?;
// let router = ash_json_api::router(ctx);
// let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
// axum::serve(listener, router).await?;
// ```

use std::sync::Arc;

pub use serde;

// Re-export dependencies for macro hygiene — the generated code from
// `ash-json-api-macros` references these through `ash_json_api::axum`,
// `ash_json_api::tracing`, etc., so they must be available at the
// call site without the user adding them as direct dependencies.
pub use axum;
pub use inventory;
pub use tracing;

// Re-export the extension proc macro so `resource!` can call
// `ash_json_api::__resource_extension!`.
pub use ash_json_api_macros::__resource_extension;

/// Generic JSON API response envelope.
///
/// Wraps all responses in a `{ "data": ... }` structure so the format is
/// extensible with future fields like pagination, links, or errors.
///
/// For list endpoints `T` is `Vec<R>`, for single-resource endpoints it
/// will be `R` directly.
#[derive(Debug, serde::Serialize)]
pub struct Response<T: serde::Serialize> {
    pub data: T,
}

/// A registered resource endpoint. Extension macros generate instances of this
/// struct and submit them via `inventory::submit!`. The `register` function
/// takes an existing router and context, and returns a new router with the
/// resource's endpoints added.
pub struct ResourceEndpoint {
    pub register: fn(axum::Router, Arc<ash_core::Context>) -> axum::Router,
}

inventory::collect!(ResourceEndpoint);

/// Builds an `axum::Router` containing all auto-registered JSON API endpoints.
///
/// Each resource that declared `ash_json_api` in its `extensions` block will
/// have its endpoints automatically included via `inventory` — no manual
/// route construction is needed.
pub fn router(ctx: impl Into<Arc<ash_core::Context>>) -> axum::Router {
    let ctx = ctx.into();

    let mut router = axum::Router::new();

    for endpoint in inventory::iter::<ResourceEndpoint> {
        router = (endpoint.register)(router, ctx.clone());
    }

    router
}
