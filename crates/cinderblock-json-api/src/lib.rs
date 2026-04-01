//! JSON REST API extension for cinderblock resources.
//!
//! When a resource declares `cinderblock_json_api` in its `extensions { ... }`
//! block, the [`resource!`](cinderblock_core::resource) macro generates Axum
//! route handlers and endpoint registration code. At startup, all registered
//! endpoints are automatically discovered via [`inventory`] and assembled into
//! an [`axum::Router`].
//!
//! # Extension configuration
//!
//! Inside the `extensions` block of a [`resource!`](cinderblock_core::resource)
//! invocation, declare routes and optional settings:
//!
//! ```rust,ignore
//! extensions {
//!     cinderblock_json_api {
//!         // Each `route` maps an HTTP method + path to a resource action.
//!         route = { method = GET;    path = "/";              action = all;    };
//!         route = { method = POST;   path = "/";              action = open;   };
//!         route = { method = POST;   path = "/assign";        action = assign; };
//!         route = { method = PATCH;  path = "/{primary_key}"; action = close;  };
//!         route = { method = DELETE; path = "/{primary_key}"; action = remove; };
//!
//!         // Optional: override the auto-derived base path.
//!         // Default: kebab-case of resource name segments joined by `/`.
//!         //   e.g. `Helpdesk.Support.Ticket` -> `/helpdesk/support/ticket`
//!         // base_path = "/api/v1/tickets";
//!
//!         // Optional: disable OpenAPI spec generation. Default: true.
//!         // openapi = false;
//!     };
//! }
//! ```
//!
//! ## Route configuration
//!
//! | Field | Required | Description |
//! |---|---|---|
//! | `method` | yes | HTTP method: `GET`, `POST`, `PATCH`, `PUT`, or `DELETE` |
//! | `path` | yes | Path relative to the base path. Use `/{primary_key}` for routes that operate on a single resource. |
//! | `action` | yes | Name of a declared action on the resource. Must match the action kind (e.g. `GET` for `read`, `POST` for `create`). |
//!
//! The action name must refer to an action declared in the resource's `actions`
//! block. Duplicate method + path combinations are rejected at compile time.
//!
//! ## Route behavior by action kind
//!
//! - **Read** (`GET`): query parameters are deserialized into the action's
//!   `Arguments` struct. Returns `{ "data": [...] }`.
//! - **Create** (`POST`): JSON body is deserialized into the action's `Input`
//!   struct. Returns `{ "data": <resource> }`.
//! - **Update** (`PATCH`/`PUT`): primary key is extracted from the URL path,
//!   JSON body is deserialized into the action's `Input` struct. Returns
//!   `{ "data": <resource> }`.
//! - **Destroy** (`DELETE`): primary key is extracted from the URL path.
//!   Returns `{ "data": <resource> }` with the deleted resource.
//!
//! All responses are wrapped in a [`Response`] envelope (`{ "data": ... }`).
//!
//! ## OpenAPI and Swagger UI
//!
//! By default, the extension generates an OpenAPI spec fragment for each
//! resource. These fragments are merged and served at `GET /openapi.json`.
//!
//! When the `swagger-ui` feature is enabled, a Swagger UI is mounted at
//! `/swagger-ui`. This can be toggled off via [`RouterConfig::swagger_ui`].
//!
//! ## Custom types in OpenAPI schemas
//!
//! The generated OpenAPI schemas use the [`FieldSchema`] trait to produce
//! schemas for each attribute type. Built-in types (`String`, integers, `bool`,
//! `Uuid`) have implementations provided. For custom types (like enums),
//! derive [`utoipa::ToSchema`] and bridge it with [`impl_field_schema!`]:
//!
//! ```rust,ignore
//! #[derive(Debug, Clone, Serialize, Deserialize, cinderblock_json_api::utoipa::ToSchema)]
//! enum TicketStatus {
//!     Open,
//!     Closed,
//! }
//!
//! cinderblock_json_api::impl_field_schema!(TicketStatus);
//! ```
//!
//! # Building the router
//!
//! Use [`router()`] for the common case, or [`RouterConfig`] for more control:
//!
//! ```rust,ignore
//! let ctx = cinderblock_core::Context::new();
//!
//! // Simple — all defaults.
//! let app = cinderblock_json_api::router(ctx);
//!
//! // Or configure options like Swagger UI.
//! let app = cinderblock_json_api::RouterConfig::new(ctx)
//!     .swagger_ui(false)
//!     .build();
//!
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, app).await?;
//! ```
//!
//! # Full example
//!
//! ```rust,ignore
//! use cinderblock_core::{Context, resource, serde::{Deserialize, Serialize}};
//! use uuid::Uuid;
//!
//! resource! {
//!     name = Helpdesk.Support.Ticket;
//!
//!     attributes {
//!         ticket_id Uuid {
//!             primary_key true;
//!             writable false;
//!             default || Uuid::new_v4();
//!         }
//!         subject String;
//!         status TicketStatus;
//!     }
//!
//!     actions {
//!         read all {
//!             argument { status: Option<TicketStatus> };
//!             filter { status == arg(status) };
//!         };
//!         create open;
//!         update close {
//!             accept [];
//!             change_ref |ticket| { ticket.status = TicketStatus::Closed; };
//!         };
//!         destroy remove;
//!     }
//!
//!     extensions {
//!         cinderblock_json_api {
//!             route = { method = GET;    path = "/";              action = all;    };
//!             route = { method = POST;   path = "/";              action = open;   };
//!             route = { method = PATCH;  path = "/{primary_key}"; action = close;  };
//!             route = { method = DELETE; path = "/{primary_key}"; action = remove; };
//!         };
//!     }
//! }
//!
//! #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq,
//!          cinderblock_json_api::utoipa::ToSchema)]
//! enum TicketStatus { #[default] Open, Closed }
//! cinderblock_json_api::impl_field_schema!(TicketStatus);
//!
//! #[tokio::main]
//! async fn main() -> cinderblock_core::Result<()> {
//!     let ctx = Context::new();
//!     let router = cinderblock_json_api::router(ctx);
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//!     axum::serve(listener, router).await?;
//!     Ok(())
//! }
//! ```

use std::sync::Arc;

pub use serde;

// Re-export dependencies for macro hygiene — the generated code from
// `cinderblock-json-api-macros` references these through `cinderblock_json_api::axum`,
// `cinderblock_json_api::tracing`, etc., so they must be available at the
// call site without the user adding them as direct dependencies.
pub use axum;
pub use inventory;
pub use tracing;
pub use utoipa;

// Re-export the extension proc macro so `resource!` can call
// `cinderblock_json_api::__resource_extension!`.
pub use cinderblock_json_api_macros::__resource_extension;

/// Helper trait that provides OpenAPI schema generation for types used as
/// resource attribute fields.
///
/// This exists because `utoipa::PartialSchema` is a foreign trait, so we
/// can't impl it for foreign types like `uuid::Uuid` due to orphan rules.
/// The extension macro generates calls to
/// `<Type as cinderblock_json_api::FieldSchema>::field_schema()` instead of
/// `<Type as utoipa::PartialSchema>::schema()`.
///
/// Types that derive `utoipa::ToSchema` (which implies `PartialSchema`)
/// can use the blanket impl via the `partial_schema_field_schema!` macro.
/// Common built-in types (`String`, integers, `bool`, `Uuid`) have
/// explicit impls provided here.
pub trait FieldSchema {
    fn field_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>;
}

/// Implements `FieldSchema` for types that already have `PartialSchema`.
///
/// Users call this for their own types that derive `ToSchema`:
/// ```rust,ignore
/// #[derive(utoipa::ToSchema)]
/// enum TicketStatus { Open, Closed }
/// cinderblock_json_api::impl_field_schema!(TicketStatus);
/// ```
#[macro_export]
macro_rules! impl_field_schema {
    ($ty:ty) => {
        impl $crate::FieldSchema for $ty {
            fn field_schema(
            ) -> $crate::utoipa::openapi::RefOr<$crate::utoipa::openapi::schema::Schema> {
                <$ty as $crate::utoipa::PartialSchema>::schema()
            }
        }
    };
}

// # Built-in FieldSchema implementations
//
// These cover the common Rust types that appear as resource attribute
// fields. The schemas match what utoipa's built-in `ComposeSchema` impls
// would produce.

macro_rules! impl_field_schema_string {
    ($($ty:ty),*) => {
        $(
            impl FieldSchema for $ty {
                fn field_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                    use utoipa::openapi::schema::{ObjectBuilder, SchemaType, Type};
                    ObjectBuilder::new()
                        .schema_type(SchemaType::new(Type::String))
                        .into()
                }
            }
        )*
    };
}

macro_rules! impl_field_schema_integer {
    ($($ty:ty => $format:expr),*) => {
        $(
            impl FieldSchema for $ty {
                fn field_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                    use utoipa::openapi::schema::{ObjectBuilder, SchemaType, SchemaFormat, Type};
                    ObjectBuilder::new()
                        .schema_type(SchemaType::new(Type::Integer))
                        .format(Some(SchemaFormat::KnownFormat($format)))
                        .into()
                }
            }
        )*
    };
}

macro_rules! impl_field_schema_number {
    ($($ty:ty => $format:expr),*) => {
        $(
            impl FieldSchema for $ty {
                fn field_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                    use utoipa::openapi::schema::{ObjectBuilder, SchemaType, SchemaFormat, Type};
                    ObjectBuilder::new()
                        .schema_type(SchemaType::new(Type::Number))
                        .format(Some(SchemaFormat::KnownFormat($format)))
                        .into()
                }
            }
        )*
    };
}

impl_field_schema_string!(String);

impl FieldSchema for bool {
    fn field_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{ObjectBuilder, SchemaType, Type};
        ObjectBuilder::new()
            .schema_type(SchemaType::new(Type::Boolean))
            .into()
    }
}

impl_field_schema_integer!(
    i8 => utoipa::openapi::KnownFormat::Int32,
    i16 => utoipa::openapi::KnownFormat::Int32,
    i32 => utoipa::openapi::KnownFormat::Int32,
    i64 => utoipa::openapi::KnownFormat::Int64,
    u8 => utoipa::openapi::KnownFormat::Int32,
    u16 => utoipa::openapi::KnownFormat::Int32,
    u32 => utoipa::openapi::KnownFormat::Int32,
    u64 => utoipa::openapi::KnownFormat::Int64,
    isize => utoipa::openapi::KnownFormat::Int64,
    usize => utoipa::openapi::KnownFormat::Int64
);

impl_field_schema_number!(
    f32 => utoipa::openapi::KnownFormat::Float,
    f64 => utoipa::openapi::KnownFormat::Double
);

impl FieldSchema for uuid::Uuid {
    fn field_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{ObjectBuilder, SchemaFormat, SchemaType, Type};
        ObjectBuilder::new()
            .schema_type(SchemaType::new(Type::String))
            .format(Some(SchemaFormat::KnownFormat(
                utoipa::openapi::KnownFormat::Uuid,
            )))
            .into()
    }
}

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

/// JSON API response envelope for paginated list endpoints.
///
/// Returns `{ "data": [...], "meta": { page, per_page, total, total_pages } }`.
/// Used by paged read action handlers instead of the plain [`Response`] envelope.
#[derive(Debug, serde::Serialize)]
pub struct PaginatedResponse<T: serde::Serialize> {
    pub data: Vec<T>,
    pub meta: PaginationMeta,
}

/// Pagination metadata included in [`PaginatedResponse`].
#[derive(Debug, serde::Serialize)]
pub struct PaginationMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u32,
}

// # PartialSchema / ToSchema for Response<T>
//
// Manual implementations so the generated OpenAPI spec can describe the
// `{ "data": ... }` envelope without requiring a derive on a struct that
// has a generic type parameter. The schema delegates to `T`'s schema for
// the `data` property.
impl<T> utoipa::PartialSchema for Response<T>
where
    T: serde::Serialize + utoipa::PartialSchema,
{
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{ObjectBuilder, SchemaType, Type};

        ObjectBuilder::new()
            .schema_type(SchemaType::new(Type::Object))
            .property("data", T::schema())
            .required("data")
            .into()
    }
}

impl<T> utoipa::ToSchema for Response<T>
where
    T: serde::Serialize + utoipa::PartialSchema,
{
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("Response")
    }
}

// # PartialSchema / ToSchema for PaginatedResponse<T>
//
// Describes the `{ "data": [...], "meta": {...} }` shape for OpenAPI specs.
impl<T> utoipa::PartialSchema for PaginatedResponse<T>
where
    T: serde::Serialize + utoipa::PartialSchema,
{
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{
            ArrayBuilder, ObjectBuilder, SchemaFormat, SchemaType, Type,
        };

        let meta_schema = ObjectBuilder::new()
            .schema_type(SchemaType::new(Type::Object))
            .property(
                "page",
                ObjectBuilder::new()
                    .schema_type(SchemaType::new(Type::Integer))
                    .format(Some(SchemaFormat::KnownFormat(
                        utoipa::openapi::KnownFormat::Int32,
                    ))),
            )
            .required("page")
            .property(
                "per_page",
                ObjectBuilder::new()
                    .schema_type(SchemaType::new(Type::Integer))
                    .format(Some(SchemaFormat::KnownFormat(
                        utoipa::openapi::KnownFormat::Int32,
                    ))),
            )
            .required("per_page")
            .property(
                "total",
                ObjectBuilder::new()
                    .schema_type(SchemaType::new(Type::Integer))
                    .format(Some(SchemaFormat::KnownFormat(
                        utoipa::openapi::KnownFormat::Int64,
                    ))),
            )
            .required("total")
            .property(
                "total_pages",
                ObjectBuilder::new()
                    .schema_type(SchemaType::new(Type::Integer))
                    .format(Some(SchemaFormat::KnownFormat(
                        utoipa::openapi::KnownFormat::Int32,
                    ))),
            )
            .required("total_pages");

        ObjectBuilder::new()
            .schema_type(SchemaType::new(Type::Object))
            .property("data", ArrayBuilder::new().items(T::schema()))
            .required("data")
            .property("meta", meta_schema)
            .required("meta")
            .into()
    }
}

impl<T> utoipa::ToSchema for PaginatedResponse<T>
where
    T: serde::Serialize + utoipa::PartialSchema,
{
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("PaginatedResponse")
    }
}

/// A registered resource endpoint. Extension macros generate instances of this
/// struct and submit them via `inventory::submit!`. The `register` function
/// takes an existing router and context, and returns a new router with the
/// resource's endpoints added.
///
/// The optional `openapi` function returns an OpenAPI spec fragment for the
/// resource's endpoints. When present, the router builder merges all fragments
/// into a single spec served at `/openapi.json`.
pub struct ResourceEndpoint {
    pub register: fn(axum::Router, Arc<cinderblock_core::Context>) -> axum::Router,
    pub openapi: Option<fn() -> utoipa::openapi::OpenApi>,
}

inventory::collect!(ResourceEndpoint);

/// Configuration builder for the JSON API router.
///
/// Allows controlling optional features like Swagger UI before building
/// the final `axum::Router`.
///
/// ```rust,ignore
/// let router = cinderblock_json_api::RouterConfig::new(ctx)
///     .swagger_ui(true)
///     .build();
/// ```
pub struct RouterConfig {
    ctx: Arc<cinderblock_core::Context>,
    swagger_ui: bool,
}

impl RouterConfig {
    pub fn new(ctx: impl Into<Arc<cinderblock_core::Context>>) -> Self {
        Self {
            ctx: ctx.into(),
            swagger_ui: true,
        }
    }

    /// Enable or disable the Swagger UI endpoint at `/swagger-ui`.
    /// Only takes effect when the `utoipa-swagger-ui` feature is enabled.
    /// Default: `true`.
    pub fn swagger_ui(mut self, enabled: bool) -> Self {
        self.swagger_ui = enabled;
        self
    }

    pub fn build(self) -> axum::Router {
        let mut router = axum::Router::new();

        // # Endpoint registration + OpenAPI spec collection
        //
        // Each resource that declared `cinderblock_json_api` in its extensions block
        // contributes both route handlers and an optional OpenAPI spec
        // fragment. We collect the fragments and merge them afterward.
        let mut openapi_specs: Vec<utoipa::openapi::OpenApi> = Vec::new();

        for endpoint in inventory::iter::<ResourceEndpoint> {
            router = (endpoint.register)(router, self.ctx.clone());

            if let Some(openapi_fn) = endpoint.openapi {
                openapi_specs.push(openapi_fn());
            }
        }

        // # OpenAPI spec merging
        //
        // Build a base spec and merge each resource's fragment into it.
        // The merged spec is served at GET /openapi.json.
        if !openapi_specs.is_empty() {
            let mut merged = utoipa::openapi::OpenApiBuilder::new()
                .info(
                    utoipa::openapi::InfoBuilder::new()
                        .title("Cinderblock JSON API")
                        .version("0.1.0")
                        .build(),
                )
                .build();

            for spec in openapi_specs {
                merged.merge(spec);
            }

            // # Swagger UI
            //
            // When the `swagger-ui` feature is enabled and the user hasn't
            // disabled it, mount the Swagger UI at `/swagger-ui`. The
            // SwaggerUi widget also serves the spec at `/openapi.json`.
            #[cfg(feature = "swagger-ui")]
            if self.swagger_ui {
                router = router.merge(
                    utoipa_swagger_ui::SwaggerUi::new("/swagger-ui").url("/openapi.json", merged),
                );
            }

            #[cfg(not(feature = "swagger-ui"))]
            let _ = self.swagger_ui;
        }

        router
    }
}

/// Builds an `axum::Router` containing all auto-registered JSON API endpoints.
///
/// This is a convenience wrapper around `RouterConfig::new(ctx).build()`.
/// Each resource that declared `cinderblock_json_api` in its `extensions` block will
/// have its endpoints automatically included via `inventory` — no manual
/// route construction is needed.
pub fn router(ctx: impl Into<Arc<cinderblock_core::Context>>) -> axum::Router {
    RouterConfig::new(ctx).build()
}
