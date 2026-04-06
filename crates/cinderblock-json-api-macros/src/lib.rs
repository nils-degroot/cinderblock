// # JSON API Extension Proc Macro
//
// This proc macro is invoked by the `resource!` macro when a resource declares
// `cinderblock_json_api` in its `extensions { ... }` block. It receives the full
// resource DSL tokens plus the extension-specific config, and generates:
//
//   1. A route registration function that wires up the resource's endpoints
//   2. An `inventory::submit!` call that auto-registers the endpoints so
//      `cinderblock_json_api::router()` can discover them without manual wiring
//   3. (Optional) `PartialSchema`/`ToSchema` impls and an OpenAPI spec
//      function for the resource and its input structs
//
// # Route declaration
//
// Routes must be explicitly declared — there is no auto-generation. Each
// route maps an HTTP method + path to an action declared on the resource.
// The action kind (read/create/update/destroy) is inferred by looking up
// the action name in the resource definition.
//
// # Config syntax
//
// ```text
// cinderblock_json_api {
//     base_path = "/api/v1/tickets";    // optional, defaults to kebab-case of resource name
//
//     route = { method = GET; path = "/"; action = all; };
//     route = { method = POST; path = "/"; action = open; };
//     route = { method = PATCH; path = "/{primary_key}/close"; action = close; };
//     route = { method = DELETE; path = "/{primary_key}"; action = remove; };
//
//     openapi = true;                   // optional, defaults to true
// };
//
// cinderblock_json_api {};              // no routes = silent no-op
// ```

use std::collections::HashSet;

use cinderblock_extension_api::{
    Accept, ExtensionMacroInput, ResourceActionInputKind, util::is_optional,
};
use syn::{Ident, LitBool, LitStr, Token, Type, braced, parse::Parse};

/// Supported HTTP methods for route declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HttpMethod {
    Get,
    Post,
    Patch,
    Put,
    Delete,
}

impl HttpMethod {
    /// Returns the method name as an uppercase string (e.g., `"GET"`).
    fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Patch => "PATCH",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }

    /// Returns the corresponding `axum::routing::*` function as a token stream.
    fn axum_routing_fn(&self) -> proc_macro2::TokenStream {
        match self {
            Self::Get => quote::quote! { cinderblock_json_api::axum::routing::get },
            Self::Post => quote::quote! { cinderblock_json_api::axum::routing::post },
            Self::Patch => quote::quote! { cinderblock_json_api::axum::routing::patch },
            Self::Put => quote::quote! { cinderblock_json_api::axum::routing::put },
            Self::Delete => quote::quote! { cinderblock_json_api::axum::routing::delete },
        }
    }

    /// Returns the corresponding `utoipa` `HttpMethod` variant as a token stream.
    fn openapi_http_method(&self) -> proc_macro2::TokenStream {
        match self {
            Self::Get => {
                quote::quote! { cinderblock_json_api::utoipa::openapi::path::HttpMethod::Get }
            }
            Self::Post => {
                quote::quote! { cinderblock_json_api::utoipa::openapi::path::HttpMethod::Post }
            }
            Self::Patch => {
                quote::quote! { cinderblock_json_api::utoipa::openapi::path::HttpMethod::Patch }
            }
            Self::Put => {
                quote::quote! { cinderblock_json_api::utoipa::openapi::path::HttpMethod::Put }
            }
            Self::Delete => {
                quote::quote! { cinderblock_json_api::utoipa::openapi::path::HttpMethod::Delete }
            }
        }
    }
}

impl Parse for HttpMethod {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match ident.to_string().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PATCH" => Ok(Self::Patch),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            got => Err(syn::Error::new(
                ident.span(),
                format!(
                    "unsupported HTTP method `{got}`, \
                     expected GET, POST, PATCH, PUT, or DELETE"
                ),
            )),
        }
    }
}

/// A single route declaration mapping an HTTP method + path to a resource action.
///
/// Parsed from `route = { method = GET; path = "/"; action = all; };`.
struct RouteDecl {
    method: HttpMethod,
    /// URL path relative to `base_path` (e.g., "/" or "/{primary_key}/close").
    path: LitStr,
    /// Name of the action on the resource (e.g., `all`, `open`, `close`).
    action: Ident,
    /// Span of the `method` field for error reporting on duplicates.
    method_span: proc_macro2::Span,
}

impl Parse for RouteDecl {
    /// Parses the braced body of a route declaration:
    /// `{ method = GET; path = "/"; action = all; }`
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut method: Option<(HttpMethod, proc_macro2::Span)> = None;
        let mut path: Option<LitStr> = None;
        let mut action: Option<Ident> = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let _: Token![=] = input.parse()?;

            match key.to_string().as_str() {
                "method" => {
                    let span = input.span();
                    let value: HttpMethod = input.parse()?;
                    method = Some((value, span));
                }
                "path" => {
                    path = Some(input.parse()?);
                }
                "action" => {
                    action = Some(input.parse()?);
                }
                got => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unexpected route field `{got}`, expected method, path, or action"),
                    ));
                }
            }

            let _: Token![;] = input.parse()?;
        }

        let (method, method_span) = method.ok_or_else(|| {
            syn::Error::new(input.span(), "route declaration missing `method` field")
        })?;
        let path = path.ok_or_else(|| {
            syn::Error::new(input.span(), "route declaration missing `path` field")
        })?;
        let action = action.ok_or_else(|| {
            syn::Error::new(input.span(), "route declaration missing `action` field")
        })?;

        Ok(RouteDecl {
            method,
            path,
            action,
            method_span,
        })
    }
}

/// Extension-specific configuration parsed from inside the `config = { ... }`
/// block.
///
/// Routes are explicitly declared — an empty config means zero endpoints
/// are registered (silent no-op).
struct JsonApiConfig {
    /// Optional base path override. Defaults to the auto-derived kebab-case
    /// path from the resource name (e.g., `Helpdesk.Support.Ticket` →
    /// `/helpdesk/support/ticket`).
    base_path: Option<LitStr>,
    /// Explicit route declarations. Each maps an HTTP method + path to a
    /// resource action.
    routes: Vec<RouteDecl>,
    /// When set to `false`, disables OpenAPI schema and spec generation for
    /// this resource. Defaults to enabled.
    openapi: Option<bool>,
}

impl JsonApiConfig {
    /// Returns `false` only when the user explicitly set `openapi = false;`.
    fn should_openapi(&self) -> bool {
        self.openapi.unwrap_or(true)
    }
}

impl Parse for JsonApiConfig {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut config = JsonApiConfig {
            base_path: None,
            routes: Vec::new(),
            openapi: None,
        };

        while !input.is_empty() {
            // Peek at the key to determine which config field to parse,
            // without consuming it — `parse_attribute` will consume it
            // for the simple `key = value;` cases.
            let key: Ident = input.fork().parse()?;

            match key.to_string().as_str() {
                "base_path" => {
                    let (_, value) = cinderblock_extension_api::parse_attribute::<LitStr>(input)?;
                    config.base_path = Some(value);
                }
                "route" => {
                    // Parses `route = { method = GET; path = "/"; action = all; };`
                    let _: Ident = input.parse()?;
                    let _: Token![=] = input.parse()?;
                    let content;
                    braced!(content in input);
                    let route: RouteDecl = content.parse()?;
                    config.routes.push(route);
                    let _: Token![;] = input.parse()?;
                }
                "openapi" => {
                    let (_, value) = cinderblock_extension_api::parse_attribute::<LitBool>(input)?;
                    config.openapi = Some(value.value());
                }
                got => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unexpected cinderblock_json_api config key, got `{got}`"),
                    ));
                }
            }
        }

        Ok(config)
    }
}

/// Computes which attribute fields appear in a given action's input struct.
///
/// This replicates the field selection logic from `cinderblock-core-macros`: start
/// with all writable attributes, then narrow by `Accept::Only` if specified.
/// The returned list contains `(field_name, field_type)` pairs.
fn input_fields_for_accept<'a>(
    attributes: &'a [cinderblock_extension_api::ResourceAttributeInput],
    accept: &Accept,
) -> Vec<(&'a Ident, &'a syn::Type)> {
    let writable: Vec<_> = attributes
        .iter()
        .filter(|attr| attr.writable.value())
        .collect();

    match accept {
        Accept::Default => writable.iter().map(|a| (&a.name, &a.ty)).collect(),
        Accept::Only(idents) => {
            let names: HashSet<String> = idents.iter().map(|i| i.to_string()).collect();
            writable
                .iter()
                .filter(|a| names.contains(&a.name.to_string()))
                .map(|a| (&a.name, &a.ty))
                .collect()
        }
    }
}

/// Extracts the inner `T` from an `Option<T>` type.
///
/// Returns `None` if the type is not an `Option` or doesn't have exactly one
/// generic argument.
fn extract_option_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        let last_seg = type_path.path.segments.last()?;
        if last_seg.ident != "Option" {
            return None;
        }
        if let syn::PathArguments::AngleBracketed(args) = &last_seg.arguments
            && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
        {
            return Some(inner);
        }
    }
    None
}

#[proc_macro]
pub fn __resource_extension(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(item as ExtensionMacroInput<JsonApiConfig>);

    let resource = &input.resource;
    let config = &input.config;

    // Derive the resource struct name from the last segment of the dotted name.
    let ident = resource
        .name
        .last()
        .expect("resource name must have at least one segment");

    // # Base path derivation
    //
    // If the user specified `base_path = "/api/v1/tickets";`, use that.
    // Otherwise, derive from the resource name by converting each segment
    // to kebab-case: `Helpdesk.Support.Ticket` → `/helpdesk/support/ticket`.
    let base_path = config
        .base_path
        .as_ref()
        .map(|lit| lit.value())
        .unwrap_or_else(|| {
            format!(
                "/{}",
                resource
                    .name
                    .iter()
                    .map(|s| convert_case::ccase!(kebab, s.to_string()))
                    .collect::<Vec<_>>()
                    .join("/")
            )
        });

    // Generate a unique function name for the registration function to avoid
    // collisions when multiple resources register endpoints.
    let name_slug = resource
        .name
        .iter()
        .map(|s| s.to_string().to_lowercase())
        .collect::<Vec<_>>()
        .join("_");

    let register_fn_name = Ident::new(&format!("__register_json_api_{name_slug}"), ident.span());

    // # Validation
    //
    // Validate all route declarations before generating code:
    //   - Each route's action must exist in the resource's action list
    //   - No duplicate method + path combinations
    let mut seen_routes: HashSet<(&str, String)> = HashSet::new();

    for route in &config.routes {
        let action_name_str = route.action.to_string();

        if !resource.actions.iter().any(|a| a.raw_name == route.action) {
            return syn::Error::new(
                route.action.span(),
                format!(
                    "route references unknown action `{action_name_str}` — \
                     it must be declared in the resource's `actions {{ ... }}` block"
                ),
            )
            .to_compile_error()
            .into();
        }

        let route_key = (route.method.as_str(), route.path.value());
        if !seen_routes.insert(route_key.clone()) {
            return syn::Error::new(
                route.method_span,
                format!(
                    "duplicate route: {} {} is declared more than once",
                    route_key.0, route_key.1
                ),
            )
            .to_compile_error()
            .into();
        }
    }

    // # Route generation
    //
    // For each declared route, we look up the action in the resource
    // definition to determine its kind (read/create/update/destroy), then
    // generate the appropriate handler with the right extractors and
    // response types.
    let route_registrations: Vec<_> = config
        .routes
        .iter()
        .map(|route| {
            let action_name_str = route.action.to_string();
            let full_path = format!("{base_path}{}", route.path.value());
            let method_str = route.method.as_str();

            let action_type_name = convert_case::ccase!(pascal, &action_name_str);
            let action_type = Ident::new(&action_type_name, route.action.span());

            let args_type =
                Ident::new(&format!("{action_type_name}Arguments"), route.action.span());

            // Look up the action definition — already validated above.
            let action_def = resource
                .actions
                .iter()
                .find(|a| a.raw_name == route.action)
                .expect("action existence validated above");

            let pre_flight_trace = quote::quote! {
                cinderblock_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    "handling read request"
                );
            };

            let error_trace = quote::quote! {
                cinderblock_json_api::tracing::error!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    error = %err,
                    "get request failed"
                );
            };

            let handler_and_method = match &action_def.kind {
                ResourceActionInputKind::Read(action_read) => {
                    let is_get = action_read.get;
                    let is_paged = action_read.paged.is_some();
                    let has_user_arguments = !action_read.arguments.is_empty();
                    let needs_arguments_struct = has_user_arguments || is_paged;

                    let handler = if is_get {
                        quote::quote! {
                            move |
                                cinderblock_json_api::axum::extract::Path(primary_key): cinderblock_json_api::axum::extract::Path<
                                    <#ident as cinderblock_core::Resource>::PrimaryKey,
                                >,
                            | {
                                let ctx = ctx.clone();
                                async move {
                                    #pre_flight_trace

                                    match cinderblock_core::read_one::<#ident, #action_type>(&ctx, &primary_key).await {
                                        Ok(result) => (
                                            cinderblock_json_api::axum::http::StatusCode::OK,
                                            cinderblock_json_api::axum::Json(
                                                cinderblock_json_api::Response { data: result },
                                            ),
                                        )
                                            .into_response(),
                                        Err(err) => {
                                            #error_trace
                                            let status = match err.data() {
                                                cinderblock_core::ReadError::NotFound { .. } =>
                                                    cinderblock_json_api::axum::http::StatusCode::NOT_FOUND,
                                                cinderblock_core::ReadError::DataLayer(_) =>
                                                    cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            };
                                            (status, err.to_string()).into_response()
                                        }
                                    }
                                }
                            }
                        }
                    } else if is_paged {
                        quote::quote! {
                            move |
                                cinderblock_json_api::axum::extract::Query(args): cinderblock_json_api::axum::extract::Query<#args_type>,
                            | {
                                let ctx = ctx.clone();
                                async move {
                                    #pre_flight_trace

                                    match cinderblock_core::read::<#ident, #action_type>(&ctx, &args).await {
                                        Ok(result) => (
                                            cinderblock_json_api::axum::http::StatusCode::OK,
                                            cinderblock_json_api::axum::Json(
                                                cinderblock_json_api::PaginatedResponse {
                                                    data: result.data,
                                                    meta: cinderblock_json_api::PaginationMeta {
                                                        page: result.meta.page,
                                                        per_page: result.meta.per_page,
                                                        total: result.meta.total,
                                                        total_pages: result.meta.total_pages,
                                                    },
                                                },
                                            ),
                                        )
                                            .into_response(),
                                        Err(err) => {
                                            #error_trace
                                            let status = match err.data() {
                                                cinderblock_core::ListError::DataLayer(_) =>
                                                    cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            };
                                            (status, err.to_string()).into_response()
                                        }
                                    }
                                }
                            }
                        }
                    } else if needs_arguments_struct {
                        quote::quote! {
                            move |
                                cinderblock_json_api::axum::extract::Query(args): cinderblock_json_api::axum::extract::Query<#args_type>,
                            | {
                                let ctx = ctx.clone();
                                async move {
                                    #pre_flight_trace

                                    match cinderblock_core::read::<#ident, #action_type>(&ctx, &args).await {
                                        Ok(results) => (
                                            cinderblock_json_api::axum::http::StatusCode::OK,
                                            cinderblock_json_api::axum::Json(
                                                cinderblock_json_api::Response { data: results },
                                            ),
                                        )
                                            .into_response(),
                                        Err(err) => {
                                            #error_trace
                                            let status = match err.data() {
                                                cinderblock_core::ListError::DataLayer(_) =>
                                                    cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            };
                                            (status, err.to_string()).into_response()
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        quote::quote! {
                            move || {
                                let ctx = ctx.clone();
                                async move {
                                    #pre_flight_trace

                                    match cinderblock_core::read::<#ident, #action_type>(&ctx, &()).await {
                                        Ok(results) => (
                                            cinderblock_json_api::axum::http::StatusCode::OK,
                                            cinderblock_json_api::axum::Json(
                                                cinderblock_json_api::Response { data: results },
                                            ),
                                        )
                                            .into_response(),
                                        Err(err) => {
                                            #error_trace
                                            let status = match err.data() {
                                                cinderblock_core::ListError::DataLayer(_) =>
                                                    cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            };
                                            (status, err.to_string()).into_response()
                                        }
                                    }
                                }
                            }
                        }
                    };

                    let routing_fn = route.method.axum_routing_fn();
                    quote::quote! { #routing_fn(#handler) }
                }
                ResourceActionInputKind::Create { .. } => {
                    let input_type =
                        Ident::new(&format!("{action_type_name}Input"), route.action.span());

                    let handler = quote::quote! {
                        move |cinderblock_json_api::axum::Json(input): cinderblock_json_api::axum::Json<#input_type>| {
                            let ctx = ctx.clone();
                            async move {
                                cinderblock_json_api::tracing::info!(
                                    resource = stringify!(#ident),
                                    action = #action_name_str,
                                    "handling create request"
                                );

                                match cinderblock_core::create::<#ident, #action_type>(input, &ctx).await {
                                    Ok(created) => (
                                        cinderblock_json_api::axum::http::StatusCode::CREATED,
                                        cinderblock_json_api::axum::Json(
                                            cinderblock_json_api::Response { data: created },
                                        ),
                                    )
                                        .into_response(),
                                    Err(err) => {
                                        cinderblock_json_api::tracing::error!(
                                            resource = stringify!(#ident),
                                            action = #action_name_str,
                                            error = %err,
                                            "create request failed"
                                        );
                                        let status = match err.data() {
                                            cinderblock_core::CreateError::DataLayer(_) =>
                                                cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                        };
                                        (status, err.to_string()).into_response()
                                    }
                                }
                            }
                        }
                    };

                    let routing_fn = route.method.axum_routing_fn();
                    quote::quote! { #routing_fn(#handler) }
                }
                ResourceActionInputKind::Update(_) => {
                    let input_type =
                        Ident::new(&format!("{action_type_name}Input"), route.action.span());

                    let handler = quote::quote! {
                        move |
                            cinderblock_json_api::axum::extract::Path(primary_key): cinderblock_json_api::axum::extract::Path<
                                <#ident as cinderblock_core::Resource>::PrimaryKey,
                            >,
                            cinderblock_json_api::axum::Json(input): cinderblock_json_api::axum::Json<#input_type>,
                        | {
                            let ctx = ctx.clone();
                            async move {
                                cinderblock_json_api::tracing::info!(
                                    resource = stringify!(#ident),
                                    action = #action_name_str,
                                    %primary_key,
                                    "handling update request"
                                );

                                match cinderblock_core::update::<#ident, #action_type>(
                                    &primary_key,
                                    input,
                                    &ctx,
                                )
                                .await
                                {
                                    Ok(updated) => (
                                        cinderblock_json_api::axum::http::StatusCode::OK,
                                        cinderblock_json_api::axum::Json(
                                            cinderblock_json_api::Response { data: updated },
                                        ),
                                    )
                                        .into_response(),
                                    Err(err) => {
                                        cinderblock_json_api::tracing::error!(
                                            resource = stringify!(#ident),
                                            action = #action_name_str,
                                            error = %err,
                                            "update request failed"
                                        );
                                        let status = match err.data() {
                                            cinderblock_core::UpdateError::NotFound { .. } =>
                                                cinderblock_json_api::axum::http::StatusCode::NOT_FOUND,
                                            cinderblock_core::UpdateError::DataLayer(_) =>
                                                cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                        };
                                        (status, err.to_string()).into_response()
                                    }
                                }
                            }
                        }
                    };

                    let routing_fn = route.method.axum_routing_fn();
                    quote::quote! { #routing_fn(#handler) }
                }
                ResourceActionInputKind::Destroy => {
                    let handler = quote::quote! {
                        move |
                            cinderblock_json_api::axum::extract::Path(primary_key): cinderblock_json_api::axum::extract::Path<
                                <#ident as cinderblock_core::Resource>::PrimaryKey,
                            >,
                        | {
                            let ctx = ctx.clone();
                            async move {
                                cinderblock_json_api::tracing::info!(
                                    resource = stringify!(#ident),
                                    action = #action_name_str,
                                    %primary_key,
                                    "handling destroy request"
                                );

                                match cinderblock_core::destroy::<#ident, #action_type>(
                                    &primary_key,
                                    &ctx,
                                )
                                .await
                                {
                                    Ok(_) => cinderblock_json_api::axum::http::StatusCode::NO_CONTENT
                                        .into_response(),
                                    Err(err) => {
                                        cinderblock_json_api::tracing::error!(
                                            resource = stringify!(#ident),
                                            action = #action_name_str,
                                            error = %err,
                                            "destroy request failed"
                                        );
                                        let status = match err.data() {
                                            cinderblock_core::DestroyError::NotFound { .. } =>
                                                cinderblock_json_api::axum::http::StatusCode::NOT_FOUND,
                                            cinderblock_core::DestroyError::DataLayer(_) =>
                                                cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                        };
                                        (status, err.to_string()).into_response()
                                    }
                                }
                            }
                        }
                    };

                    let routing_fn = route.method.axum_routing_fn();
                    quote::quote! { #routing_fn(#handler) }
                }
            };

            quote::quote! {
                {
                    let ctx = ctx.clone();
                    cinderblock_json_api::tracing::info!(
                        resource = stringify!(#ident),
                        action = #action_name_str,
                        method = #method_str,
                        route = #full_path,
                        "registering JSON API endpoint"
                    );
                    router = router.route(
                        #full_path,
                        #handler_and_method,
                    );
                }
            }
        })
        .collect();

    // # OpenAPI generation
    //
    // When `openapi` is not explicitly disabled, we generate:
    //
    //   1. `PartialSchema` impl for the resource struct — builds an object
    //      schema from all attributes
    //   2. `PartialSchema` impls for each enabled action's input struct —
    //      replicates the field selection logic from `cinderblock-core-macros`
    //   3. An `openapi_fn` that builds an `OpenApi` spec fragment with
    //      component schemas and path items for all enabled endpoints
    //
    // User-defined types (like `TicketStatus`) must implement `PartialSchema`
    // themselves — we delegate via `<FieldType as PartialSchema>::schema()`.
    let openapi_impls = if config.should_openapi() {
        let ident_str = ident.to_string();

        // # Resource struct schema
        //
        // Build an ObjectBuilder with a `.property()` + `.required()` call
        // for each attribute. Each field's type schema is obtained via
        // `<Type as PartialSchema>::schema()`.
        let resource_schema_properties: Vec<_> = resource
            .attributes
            .iter()
            .map(|attr| {
                let field_name = attr.name.to_string();
                let field_type = &attr.ty;

                let required_clause = if is_optional(field_type) {
                    quote::quote! {}
                } else {
                    quote::quote! { .required(#field_name) }
                };

                quote::quote! {
                    .property(
                        #field_name,
                        <#field_type as cinderblock_json_api::FieldSchema>::field_schema(),
                    )
                    #required_clause
                }
            })
            .collect();

        let resource_schema_impl = quote::quote! {
            impl cinderblock_json_api::utoipa::PartialSchema for #ident {
                fn schema() -> cinderblock_json_api::utoipa::openapi::RefOr<
                    cinderblock_json_api::utoipa::openapi::schema::Schema,
                > {
                    cinderblock_json_api::utoipa::openapi::schema::ObjectBuilder::new()
                        .schema_type(
                            cinderblock_json_api::utoipa::openapi::schema::SchemaType::new(
                                cinderblock_json_api::utoipa::openapi::schema::Type::Object,
                            ),
                        )
                        #(#resource_schema_properties)*
                        .into()
                }
            }

            impl cinderblock_json_api::utoipa::ToSchema for #ident {
                fn name() -> ::std::borrow::Cow<'static, str> {
                    ::std::borrow::Cow::Borrowed(#ident_str)
                }
            }
        };

        // # Input struct schemas
        //
        // For each routed action that has an input struct (create and update
        // actions), generate `PartialSchema` + `ToSchema` impls. Only actions
        // that are actually routed get schemas.
        let routed_action_names: HashSet<String> =
            config.routes.iter().map(|r| r.action.to_string()).collect();

        let input_schema_impls: Vec<_> = resource
            .actions
            .iter()
            .filter_map(|action| {
                let action_name_str = action.raw_name.to_string();
                if !routed_action_names.contains(&action_name_str) {
                    return None;
                }

                let action_type_name = convert_case::ccase!(pascal, &action_name_str);
                let input_type =
                    Ident::new(&format!("{action_type_name}Input"), action.raw_name.span());
                let input_type_str = format!("{action_type_name}Input");

                let accept = match &action.kind {
                    ResourceActionInputKind::Create(create) => &create.accept,
                    ResourceActionInputKind::Update(update) => &update.accept,
                    _ => return None,
                };

                let fields = input_fields_for_accept(&resource.attributes, accept);

                let properties: Vec<_> = fields
                    .iter()
                    .map(|(name, ty)| {
                        let name_str = name.to_string();

                        let required_clause = if is_optional(ty) {
                            quote::quote! {}
                        } else {
                            quote::quote! { .required(#name_str) }
                        };

                        quote::quote! {
                            .property(
                                #name_str,
                                <#ty as cinderblock_json_api::FieldSchema>::field_schema(),
                            )
                            #required_clause
                        }
                    })
                    .collect();

                Some(quote::quote! {
                    impl cinderblock_json_api::utoipa::PartialSchema for #input_type {
                        fn schema() -> cinderblock_json_api::utoipa::openapi::RefOr<
                            cinderblock_json_api::utoipa::openapi::schema::Schema,
                        > {
                            cinderblock_json_api::utoipa::openapi::schema::ObjectBuilder::new()
                                .schema_type(
                                    cinderblock_json_api::utoipa::openapi::schema::SchemaType::new(
                                        cinderblock_json_api::utoipa::openapi::schema::Type::Object,
                                    ),
                                )
                                #(#properties)*
                                .into()
                        }
                    }

                    impl cinderblock_json_api::utoipa::ToSchema for #input_type {
                        fn name() -> ::std::borrow::Cow<'static, str> {
                            ::std::borrow::Cow::Borrowed(#input_type_str)
                        }
                    }
                })
            })
            .collect();

        // # OpenAPI spec function
        //
        // Builds a complete `OpenApi` fragment containing:
        //   - Component schemas for the resource and all input structs
        //   - Path items with operations for each declared route
        //   - Request/response body schemas referencing the components
        //   - Tags based on the resource struct name
        let openapi_fn_name = Ident::new(&format!("__openapi_json_api_{name_slug}"), ident.span());

        // Schema component registrations for the spec.
        let resource_component = {
            let ident_str_val = ident.to_string();
            quote::quote! {
                .schema(
                    #ident_str_val,
                    <#ident as cinderblock_json_api::utoipa::PartialSchema>::schema(),
                )
            }
        };

        let input_components: Vec<_> = resource
            .actions
            .iter()
            .filter_map(|action| {
                let action_name_str = action.raw_name.to_string();
                if !routed_action_names.contains(&action_name_str) {
                    return None;
                }

                let action_type_name = convert_case::ccase!(pascal, &action_name_str);
                let input_type =
                    Ident::new(&format!("{action_type_name}Input"), action.raw_name.span());
                let input_type_str = format!("{action_type_name}Input");

                // Only create/update actions have input structs.
                match &action.kind {
                    ResourceActionInputKind::Create { .. } | ResourceActionInputKind::Update(_) => {
                    }
                    _ => return None,
                }

                Some(quote::quote! {
                    .schema(
                        #input_type_str,
                        <#input_type as cinderblock_json_api::utoipa::PartialSchema>::schema(),
                    )
                })
            })
            .collect();

        // # Path items for each declared route
        //
        // Each route declaration produces one OpenAPI path item. The action
        // kind determines the response shape (read returns Vec, create/update
        // returns single, destroy returns 204).
        let ident_kebab = convert_case::ccase!(kebab, ident.to_string());

        let path_items: Vec<_> = config
            .routes
            .iter()
            .map(|route| {
                let action_name_str = route.action.to_string();
                let full_path = format!("{}{}", base_path, route.path.value());
                let action_path_kebab = convert_case::ccase!(kebab, &action_name_str);
                let http_method = route.method.openapi_http_method();
                let method_lower = route.method.as_str().to_lowercase();
                let operation_id = format!("{}-{}-{}", method_lower, ident_kebab, action_path_kebab);

                let action_def = resource
                    .actions
                    .iter()
                    .find(|a| a.raw_name == route.action)
                    .expect("action existence validated above");

                // Find the primary key type for path parameter schemas.
                let pk_type = resource
                    .attributes
                    .iter()
                    .find(|a| a.primary_key.value())
                    .map(|a| &a.ty);

                // Generate a path parameter for {primary_key} if the route
                // path contains it.
                let pk_parameter = if route.path.value().contains("{primary_key}") {
                    pk_type.map(|ty| {
                        quote::quote! {
                            .parameter(
                                cinderblock_json_api::utoipa::openapi::path::ParameterBuilder::new()
                                    .name("primary_key")
                                    .parameter_in(cinderblock_json_api::utoipa::openapi::path::ParameterIn::Path)
                                    .required(cinderblock_json_api::utoipa::openapi::Required::True)
                                    .schema(Some(<#ty as cinderblock_json_api::FieldSchema>::field_schema()))
                                    .build(),
                            )
                        }
                    })
                } else {
                    None
                };

                let action_type_name = convert_case::ccase!(pascal, &action_name_str);
                let input_type =
                    Ident::new(&format!("{action_type_name}Input"), route.action.span());

                match &action_def.kind {
                    ResourceActionInputKind::Read(action_read) => {
                        let is_get = action_read.get;
                        let is_paged = action_read.paged.is_some();

                        if is_get {
                            quote::quote! {
                                .path(
                                    #full_path,
                                    cinderblock_json_api::utoipa::openapi::PathItem::new(
                                        #http_method,
                                        cinderblock_json_api::utoipa::openapi::path::OperationBuilder::new()
                                            .operation_id(Some(#operation_id))
                                            .tag(#ident_str)
                                            .summary(Some(format!("Get {} via {}", #ident_str, #action_name_str)))
                                            #pk_parameter
                                            .response(
                                                "200",
                                                cinderblock_json_api::utoipa::openapi::ResponseBuilder::new()
                                                    .description(format!("Single {}", #ident_str))
                                                    .content(
                                                        "application/json",
                                                        cinderblock_json_api::utoipa::openapi::ContentBuilder::new()
                                                            .schema(Some(
                                                                cinderblock_json_api::utoipa::openapi::RefOr::<cinderblock_json_api::utoipa::openapi::schema::Schema>::from(
                                                                    cinderblock_json_api::utoipa::openapi::schema::ObjectBuilder::new()
                                                                        .schema_type(
                                                                            cinderblock_json_api::utoipa::openapi::schema::SchemaType::new(
                                                                                cinderblock_json_api::utoipa::openapi::schema::Type::Object,
                                                                            ),
                                                                        )
                                                                        .property(
                                                                            "data",
                                                                            <#ident as cinderblock_json_api::utoipa::PartialSchema>::schema(),
                                                                        )
                                                                        .required("data")
                                                                )
                                                            ))
                                                            .build(),
                                                    )
                                                    .build(),
                                            )
                                            .response(
                                                "404",
                                                cinderblock_json_api::utoipa::openapi::ResponseBuilder::new()
                                                    .description("Not found")
                                                    .build(),
                                            )
                                        .build(),
                                ),
                            )
                            }
                        } else {

                        // Query parameters for read action arguments.
                        //
                        // Parameter names use the original Rust field name
                        // (snake_case) so that the OpenAPI spec matches what
                        // serde actually deserializes from the query string.
                        let query_params: Vec<_> = action_read.arguments.iter().map(|arg| {
                            let arg_param_name = arg.name.to_string();
                            let is_optional = is_optional(&arg.ty);

                            let schema_type = if is_optional {
                                extract_option_inner_type(&arg.ty).unwrap_or(&arg.ty)
                            } else {
                                &arg.ty
                            };

                            let required_value = if is_optional {
                                quote::quote! { cinderblock_json_api::utoipa::openapi::Required::False }
                            } else {
                                quote::quote! { cinderblock_json_api::utoipa::openapi::Required::True }
                            };

                            quote::quote! {
                                .parameter(
                                    cinderblock_json_api::utoipa::openapi::path::ParameterBuilder::new()
                                        .name(#arg_param_name)
                                        .parameter_in(cinderblock_json_api::utoipa::openapi::path::ParameterIn::Query)
                                        .required(#required_value)
                                        .schema(Some(<#schema_type as cinderblock_json_api::FieldSchema>::field_schema()))
                                        .build(),
                                )
                            }
                        }).collect();

                        // For paged reads, add `page` and `per_page` query params
                        // to the OpenAPI spec.
                        let paged_query_params = if is_paged {
                            quote::quote! {
                                .parameter(
                                    cinderblock_json_api::utoipa::openapi::path::ParameterBuilder::new()
                                        .name("page")
                                        .parameter_in(cinderblock_json_api::utoipa::openapi::path::ParameterIn::Query)
                                        .required(cinderblock_json_api::utoipa::openapi::Required::False)
                                        .schema(Some(<u32 as cinderblock_json_api::FieldSchema>::field_schema()))
                                        .description(Some("Page number (1-indexed, default: 1)"))
                                        .build(),
                                )
                                .parameter(
                                    cinderblock_json_api::utoipa::openapi::path::ParameterBuilder::new()
                                        .name("per_page")
                                        .parameter_in(cinderblock_json_api::utoipa::openapi::path::ParameterIn::Query)
                                        .required(cinderblock_json_api::utoipa::openapi::Required::False)
                                        .schema(Some(<u32 as cinderblock_json_api::FieldSchema>::field_schema()))
                                        .description(Some("Items per page"))
                                        .build(),
                                )
                            }
                        } else {
                            quote::quote! {}
                        };

                        // Response schema differs: paged reads include meta,
                        // non-paged reads return { data: [...] }.
                        let response_schema = if is_paged {
                            quote::quote! {
                                <cinderblock_json_api::PaginatedResponse<#ident> as cinderblock_json_api::utoipa::PartialSchema>::schema()
                            }
                        } else {
                            quote::quote! {
                                cinderblock_json_api::utoipa::openapi::RefOr::<cinderblock_json_api::utoipa::openapi::schema::Schema>::from(
                                    cinderblock_json_api::utoipa::openapi::schema::ObjectBuilder::new()
                                        .schema_type(
                                            cinderblock_json_api::utoipa::openapi::schema::SchemaType::new(
                                                cinderblock_json_api::utoipa::openapi::schema::Type::Object,
                                            ),
                                        )
                                        .property(
                                            "data",
                                            cinderblock_json_api::utoipa::openapi::schema::ArrayBuilder::new()
                                                .items(<#ident as cinderblock_json_api::utoipa::PartialSchema>::schema()),
                                        )
                                        .required("data")
                                )
                            }
                        };

                        let summary_prefix = if is_paged { "Paged read" } else { "Read" };

                        quote::quote! {
                            .path(
                                #full_path,
                                cinderblock_json_api::utoipa::openapi::PathItem::new(
                                    #http_method,
                                    cinderblock_json_api::utoipa::openapi::path::OperationBuilder::new()
                                        .operation_id(Some(#operation_id))
                                        .tag(#ident_str)
                                        .summary(Some(format!("{} {} via {}", #summary_prefix, #ident_str, #action_name_str)))
                                        #pk_parameter
                                        #(#query_params)*
                                        #paged_query_params
                                        .response(
                                            "200",
                                            cinderblock_json_api::utoipa::openapi::ResponseBuilder::new()
                                                .description(format!("Filtered list of {}s", #ident_str))
                                                .content(
                                                    "application/json",
                                                    cinderblock_json_api::utoipa::openapi::ContentBuilder::new()
                                                        .schema(Some(#response_schema))
                                                        .build(),
                                                )
                                                .build(),
                                        )
                                        .build(),
                                ),
                            )
                        }
                    }
                    }
                    ResourceActionInputKind::Create(create) => {
                        let fields = input_fields_for_accept(&resource.attributes, &create.accept);
                        let body_required = !fields.is_empty();

                        quote::quote! {
                            .path(
                                #full_path,
                                cinderblock_json_api::utoipa::openapi::PathItem::new(
                                    #http_method,
                                    cinderblock_json_api::utoipa::openapi::path::OperationBuilder::new()
                                        .operation_id(Some(#operation_id))
                                        .tag(#ident_str)
                                        .summary(Some(format!("Create {} via {}", #ident_str, #action_name_str)))
                                        #pk_parameter
                                        .request_body(Some(
                                            cinderblock_json_api::utoipa::openapi::request_body::RequestBodyBuilder::new()
                                                .content(
                                                    "application/json",
                                                    cinderblock_json_api::utoipa::openapi::ContentBuilder::new()
                                                        .schema(Some(<#input_type as cinderblock_json_api::utoipa::PartialSchema>::schema()))
                                                        .build(),
                                                )
                                                .required(Some(
                                                    if #body_required {
                                                        cinderblock_json_api::utoipa::openapi::Required::True
                                                    } else {
                                                        cinderblock_json_api::utoipa::openapi::Required::False
                                                    },
                                                ))
                                                .build(),
                                        ))
                                        .response(
                                            "201",
                                            cinderblock_json_api::utoipa::openapi::ResponseBuilder::new()
                                                .description(format!("{} created", #ident_str))
                                                .content(
                                                    "application/json",
                                                    cinderblock_json_api::utoipa::openapi::ContentBuilder::new()
                                                        .schema(Some(
                                                            cinderblock_json_api::utoipa::openapi::schema::ObjectBuilder::new()
                                                                .schema_type(
                                                                    cinderblock_json_api::utoipa::openapi::schema::SchemaType::new(
                                                                        cinderblock_json_api::utoipa::openapi::schema::Type::Object,
                                                                    ),
                                                                )
                                                                .property(
                                                                    "data",
                                                                    <#ident as cinderblock_json_api::utoipa::PartialSchema>::schema(),
                                                                )
                                                                .required("data"),
                                                        ))
                                                        .build(),
                                                )
                                                .build(),
                                        )
                                        .build(),
                                ),
                            )
                        }
                    }
                    ResourceActionInputKind::Update(update) => {
                        let fields = input_fields_for_accept(&resource.attributes, &update.accept);
                        let body_required = !fields.is_empty();

                        quote::quote! {
                            .path(
                                #full_path,
                                cinderblock_json_api::utoipa::openapi::PathItem::new(
                                    #http_method,
                                    cinderblock_json_api::utoipa::openapi::path::OperationBuilder::new()
                                        .operation_id(Some(#operation_id))
                                        .tag(#ident_str)
                                        .summary(Some(format!("Update {} via {}", #ident_str, #action_name_str)))
                                        #pk_parameter
                                        .request_body(Some(
                                            cinderblock_json_api::utoipa::openapi::request_body::RequestBodyBuilder::new()
                                                .content(
                                                    "application/json",
                                                    cinderblock_json_api::utoipa::openapi::ContentBuilder::new()
                                                        .schema(Some(<#input_type as cinderblock_json_api::utoipa::PartialSchema>::schema()))
                                                        .build(),
                                                )
                                                .required(Some(
                                                    if #body_required {
                                                        cinderblock_json_api::utoipa::openapi::Required::True
                                                    } else {
                                                        cinderblock_json_api::utoipa::openapi::Required::False
                                                    },
                                                ))
                                                .build(),
                                        ))
                                        .response(
                                            "200",
                                            cinderblock_json_api::utoipa::openapi::ResponseBuilder::new()
                                                .description(format!("{} updated", #ident_str))
                                                .content(
                                                    "application/json",
                                                    cinderblock_json_api::utoipa::openapi::ContentBuilder::new()
                                                        .schema(Some(
                                                            cinderblock_json_api::utoipa::openapi::schema::ObjectBuilder::new()
                                                                .schema_type(
                                                                    cinderblock_json_api::utoipa::openapi::schema::SchemaType::new(
                                                                        cinderblock_json_api::utoipa::openapi::schema::Type::Object,
                                                                    ),
                                                                )
                                                                .property(
                                                                    "data",
                                                                    <#ident as cinderblock_json_api::utoipa::PartialSchema>::schema(),
                                                                )
                                                                .required("data"),
                                                        ))
                                                        .build(),
                                                )
                                                .build(),
                                        )
                                        .build(),
                                ),
                            )
                        }
                    }
                    ResourceActionInputKind::Destroy => {
                        quote::quote! {
                            .path(
                                #full_path,
                                cinderblock_json_api::utoipa::openapi::PathItem::new(
                                    #http_method,
                                    cinderblock_json_api::utoipa::openapi::path::OperationBuilder::new()
                                        .operation_id(Some(#operation_id))
                                        .tag(#ident_str)
                                        .summary(Some(format!("Destroy {} via {}", #ident_str, #action_name_str)))
                                        #pk_parameter
                                        .response(
                                            "204",
                                            cinderblock_json_api::utoipa::openapi::ResponseBuilder::new()
                                                .description(format!("{} destroyed", #ident_str))
                                                .build(),
                                        )
                                        .build(),
                                ),
                            )
                        }
                    }
                }
            })
            .collect();

        Some(quote::quote! {
            #resource_schema_impl
            #(#input_schema_impls)*

            fn #openapi_fn_name() -> cinderblock_json_api::utoipa::openapi::OpenApi {
                cinderblock_json_api::utoipa::openapi::OpenApiBuilder::new()
                    .components(Some(
                        cinderblock_json_api::utoipa::openapi::ComponentsBuilder::new()
                            #resource_component
                            #(#input_components)*
                            .build(),
                    ))
                    .paths(
                        cinderblock_json_api::utoipa::openapi::PathsBuilder::new()
                            #(#path_items)*
                            .build(),
                    )
                    .build()
            }
        })
    } else {
        None
    };

    // # Inventory submission
    //
    // The `openapi` field is populated when OpenAPI generation is enabled,
    // or set to `None` when the user disabled it with `openapi = false;`.
    let openapi_fn_name = Ident::new(&format!("__openapi_json_api_{name_slug}"), ident.span());

    let openapi_field = if config.should_openapi() {
        quote::quote! { openapi: Some(#openapi_fn_name) }
    } else {
        quote::quote! { openapi: None }
    };

    quote::quote! {
        fn #register_fn_name(
            mut router: cinderblock_json_api::axum::Router,
            ctx: ::std::sync::Arc<cinderblock_core::Context>,
        ) -> cinderblock_json_api::axum::Router {
            use cinderblock_json_api::axum::response::IntoResponse;

            #(#route_registrations)*

            router
        }

        #openapi_impls

        cinderblock_json_api::inventory::submit! {
            cinderblock_json_api::ResourceEndpoint {
                register: #register_fn_name,
                #openapi_field,
            }
        }
    }
    .into()
}
