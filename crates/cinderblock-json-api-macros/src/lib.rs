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
// # Supported endpoints
//
// - **read**: `GET /resource_path/:action` — returns filtered resources via
//   the named read action
// - **create**: `POST /resource_path/:action` — creates a resource via the
//   named action
// - **update**: `PATCH /resource_path/{primary_key}/:action` — updates a
//   resource via the named action
// - **destroy**: `DELETE /resource_path/{primary_key}/:action` — destroys a
//   resource via the named action
//
// # Config semantics
//
// An empty config block (`cinderblock_json_api {}`) exposes everything: all
// declared read, create, update, and destroy actions. If any of `read`,
// `create`, `update`, or `destroy` is explicitly specified, only those are
// exposed.
//
// ```text
// cinderblock_json_api {};                          // expose everything
// cinderblock_json_api { read = [all]; };           // only the "all" read action
// cinderblock_json_api { create = [open]; };        // only the "open" create action
// cinderblock_json_api { create = [open]; read = [all]; };  // read all + create open
// cinderblock_json_api { destroy = [remove]; };     // only the "remove" destroy action
// cinderblock_json_api { openapi = false; };        // expose everything, no OpenAPI
// ```

use std::collections::HashSet;

use cinderblock_extension_api::{Accept, ExtensionMacroInput, ResourceActionInputKind};
use syn::{bracketed, parse::Parse, Ident, LitBool, Token, Type};

/// Extension-specific configuration parsed from inside the `config = { ... }`
/// block.
///
/// When all fields are `None` (empty config), the extension exposes
/// everything. When any field is `Some`, only the explicitly enabled
/// endpoints are generated.
struct JsonApiConfig {
    read: Option<Vec<Ident>>,
    create: Option<Vec<Ident>>,
    update: Option<Vec<Ident>>,
    destroy: Option<Vec<Ident>>,
    /// When set to `false`, disables OpenAPI schema and spec generation for
    /// this resource. Defaults to enabled. This field is orthogonal to
    /// endpoint selection — it does not participate in `is_explicit()`.
    openapi: Option<bool>,
}

impl JsonApiConfig {
    /// Returns `true` if the user explicitly configured any endpoints,
    /// meaning we should only expose what was listed.
    ///
    /// Note: `openapi` is intentionally excluded — it controls schema
    /// generation, not endpoint selection.
    fn is_explicit(&self) -> bool {
        self.read.is_some()
            || self.create.is_some()
            || self.update.is_some()
            || self.destroy.is_some()
    }

    /// Returns `false` only when the user explicitly set `openapi = false;`.
    fn should_openapi(&self) -> bool {
        self.openapi.unwrap_or(true)
    }

    fn should_read(&self, action_name: &str) -> bool {
        if self.is_explicit() {
            self.read
                .as_ref()
                .is_some_and(|names| names.iter().any(|n| n == action_name))
        } else {
            true
        }
    }

    fn should_create(&self, action_name: &str) -> bool {
        if self.is_explicit() {
            self.create
                .as_ref()
                .is_some_and(|names| names.iter().any(|n| n == action_name))
        } else {
            true
        }
    }

    fn should_update(&self, action_name: &str) -> bool {
        if self.is_explicit() {
            self.update
                .as_ref()
                .is_some_and(|names| names.iter().any(|n| n == action_name))
        } else {
            true
        }
    }

    fn should_destroy(&self, action_name: &str) -> bool {
        if self.is_explicit() {
            self.destroy
                .as_ref()
                .is_some_and(|names| names.iter().any(|n| n == action_name))
        } else {
            true
        }
    }
}

impl Parse for JsonApiConfig {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut config = JsonApiConfig {
            read: None,
            create: None,
            update: None,
            destroy: None,
            openapi: None,
        };

        while !input.is_empty() {
            // Peek at the key to determine which config field to parse,
            // without consuming it — `parse_attribute` will consume it
            // for the simple `key = value;` cases.
            let key: Ident = input.fork().parse()?;

            match key.to_string().as_str() {
                "read" => {
                    let _: Ident = input.parse()?;
                    let _: Token![=] = input.parse()?;
                    let content;
                    bracketed!(content in input);
                    let names = content
                        .parse_terminated(Ident::parse, Token![,])?
                        .into_iter()
                        .collect();
                    config.read = Some(names);
                    let _: Token![;] = input.parse()?;
                }
                "create" => {
                    let _: Ident = input.parse()?;
                    let _: Token![=] = input.parse()?;
                    let content;
                    bracketed!(content in input);
                    let names = content
                        .parse_terminated(Ident::parse, Token![,])?
                        .into_iter()
                        .collect();
                    config.create = Some(names);
                    let _: Token![;] = input.parse()?;
                }
                "update" => {
                    let _: Ident = input.parse()?;
                    let _: Token![=] = input.parse()?;
                    let content;
                    bracketed!(content in input);
                    let names = content
                        .parse_terminated(Ident::parse, Token![,])?
                        .into_iter()
                        .collect();
                    config.update = Some(names);
                    let _: Token![;] = input.parse()?;
                }
                "destroy" => {
                    let _: Ident = input.parse()?;
                    let _: Token![=] = input.parse()?;
                    let content;
                    bracketed!(content in input);
                    let names = content
                        .parse_terminated(Ident::parse, Token![,])?
                        .into_iter()
                        .collect();
                    config.destroy = Some(names);
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

/// Checks whether a `syn::Type` is `Option<T>`.
fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        type_path
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "Option")
    } else {
        false
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

    // # Route path derivation
    //
    // The base route path is built by converting each name segment to
    // kebab-case and joining with `/`. For example,
    // `Helpdesk.Support.Ticket` becomes `/helpdesk/support/ticket` and
    // `Helpdesk.SupportTicket` becomes `/helpdesk/support-ticket`.
    let base_path = format!(
        "/{}",
        resource
            .name
            .iter()
            .map(|s| convert_case::ccase!(kebab, s.to_string()))
            .collect::<Vec<_>>()
            .join("/")
    );

    // Generate a unique function name for the registration function to avoid
    // collisions when multiple resources register endpoints.
    let name_slug = resource
        .name
        .iter()
        .map(|s| s.to_string().to_lowercase())
        .collect::<Vec<_>>()
        .join("_");

    let register_fn_name = Ident::new(&format!("__register_json_api_{name_slug}"), ident.span());

    // # Read endpoints
    //
    // GET /helpdesk/support/ticket/open-tickets → returns filtered resources
    //
    // For each read action declared on the resource, we generate a GET
    // route. The action name is converted to kebab-case for the URL path
    // segment and to PascalCase to reference the generated marker struct.
    let read_routes = resource.actions.iter().filter_map(|action| {
        let action_read = match &action.kind {
            ResourceActionInputKind::Read(action_read) => action_read,
            _ => return None,
        };

        let action_name_str = action.name.to_string();
        if !config.should_read(&action_name_str) {
            return None;
        }

        let action_path_name = convert_case::ccase!(kebab, &action_name_str);
        let route_path = format!("{}/{}", base_path, action_path_name);

        let action_type_name = convert_case::ccase!(pascal, &action_name_str);
        let action_type = Ident::new(&action_type_name, action.name.span());

        let has_arguments = !action_read.arguments.is_empty();

        // # Handler generation
        //
        // When the read action has arguments, we use axum's `Query<ArgsType>`
        // extractor to parse query parameters from the URL. When there are
        // no arguments, we pass `&()` directly.
        let handler = if has_arguments {
            let args_type = Ident::new(&format!("{action_type_name}Arguments"), action.name.span());
            quote::quote! {
                move |
                    cinderblock_json_api::axum::extract::Query(args): cinderblock_json_api::axum::extract::Query<#args_type>,
                | {
                    let ctx = ctx.clone();
                    async move {
                        cinderblock_json_api::tracing::info!(
                            resource = stringify!(#ident),
                            action = #action_name_str,
                            "handling read request"
                        );

                        match cinderblock_core::read::<#ident, #action_type>(&ctx, &args).await {
                            Ok(results) => (
                                cinderblock_json_api::axum::http::StatusCode::OK,
                                cinderblock_json_api::axum::Json(
                                    cinderblock_json_api::Response { data: results },
                                ),
                            )
                                .into_response(),
                            Err(err) => {
                                cinderblock_json_api::tracing::error!(
                                    resource = stringify!(#ident),
                                    action = #action_name_str,
                                    error = %err,
                                    "read request failed"
                                );
                                (
                                    cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                    err.to_string(),
                                )
                                    .into_response()
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
                        cinderblock_json_api::tracing::info!(
                            resource = stringify!(#ident),
                            action = #action_name_str,
                            "handling read request"
                        );

                        match cinderblock_core::read::<#ident, #action_type>(&ctx, &()).await {
                            Ok(results) => (
                                cinderblock_json_api::axum::http::StatusCode::OK,
                                cinderblock_json_api::axum::Json(
                                    cinderblock_json_api::Response { data: results },
                                ),
                            )
                                .into_response(),
                            Err(err) => {
                                cinderblock_json_api::tracing::error!(
                                    resource = stringify!(#ident),
                                    action = #action_name_str,
                                    error = %err,
                                    "read request failed"
                                );
                                (
                                    cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                    err.to_string(),
                                )
                                    .into_response()
                            }
                        }
                    }
                }
            }
        };

        Some(quote::quote! {
            {
                let ctx = ctx.clone();
                cinderblock_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    route = #route_path,
                    "registering JSON API read endpoint"
                );
                router = router.route(
                    #route_path,
                    cinderblock_json_api::axum::routing::get(#handler),
                );
            }
        })
    });

    // # Create endpoints
    //
    // POST /helpdesk/support/ticket/open → creates via the "open" action
    //
    // For each create action declared on the resource, we generate a POST
    // route. The action name is converted to kebab-case for the URL path
    // segment and to PascalCase to reference the generated marker and
    // input structs.
    let create_routes = resource.actions.iter().filter_map(|action| {
        match &action.kind {
            ResourceActionInputKind::Create { .. } => {}
            _ => return None,
        }

        let action_name_str = action.name.to_string();
        if !config.should_create(&action_name_str) {
            return None;
        }

        let action_path_name = convert_case::ccase!(kebab, &action_name_str);
        let route_path = format!("{}/{}", base_path, action_path_name);

        let action_type_name = convert_case::ccase!(pascal, &action_name_str);
        let action_type = Ident::new(&action_type_name, action.name.span());
        let input_type = Ident::new(&format!("{action_type_name}Input"), action.name.span());

        Some(quote::quote! {
            {
                let ctx = ctx.clone();
                cinderblock_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    route = #route_path,
                    "registering JSON API create endpoint"
                );
                router = router.route(
                    #route_path,
                    cinderblock_json_api::axum::routing::post(
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
                                        (
                                            cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            err.to_string(),
                                        )
                                            .into_response()
                                    }
                                }
                            }
                        },
                    ),
                );
            }
        })
    });

    // # Update endpoints
    //
    // PATCH /helpdesk/support/ticket/{primary_key}/close → updates via "close"
    //
    // The primary key is extracted from the URL path and parsed as the
    // resource's `PrimaryKey` type. The action's input struct is deserialized
    // from the JSON body.
    let update_routes = resource.actions.iter().filter_map(|action| {
        match &action.kind {
            ResourceActionInputKind::Update(_) => {}
            _ => return None,
        }

        let action_name_str = action.name.to_string();
        if !config.should_update(&action_name_str) {
            return None;
        }

        let action_path_name = convert_case::ccase!(kebab, &action_name_str);
        let route_path = format!("{}/{{primary_key}}/{}", base_path, action_path_name);

        let action_type_name = convert_case::ccase!(pascal, &action_name_str);
        let action_type = Ident::new(&action_type_name, action.name.span());
        let input_type = Ident::new(&format!("{action_type_name}Input"), action.name.span());

        Some(quote::quote! {
            {
                let ctx = ctx.clone();
                cinderblock_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    route = #route_path,
                    "registering JSON API update endpoint"
                );
                router = router.route(
                    #route_path,
                    cinderblock_json_api::axum::routing::patch(
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
                                        (
                                            cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            err.to_string(),
                                        )
                                            .into_response()
                                    }
                                }
                            }
                        },
                    ),
                );
            }
        })
    });

    // # Destroy endpoints
    //
    // DELETE /helpdesk/support/ticket/{primary_key}/remove → destroys via "remove"
    //
    // The primary key is extracted from the URL path. The response is
    // `204 No Content` with an empty body — the deleted resource is not
    // returned over the wire (even though `destroy()` returns it internally).
    let destroy_routes = resource.actions.iter().filter_map(|action| {
        match &action.kind {
            ResourceActionInputKind::Destroy => {}
            _ => return None,
        }

        let action_name_str = action.name.to_string();
        if !config.should_destroy(&action_name_str) {
            return None;
        }

        let action_path_name = convert_case::ccase!(kebab, &action_name_str);
        let route_path = format!("{}/{{primary_key}}/{}", base_path, action_path_name);

        let action_type_name = convert_case::ccase!(pascal, &action_name_str);
        let action_type = Ident::new(&action_type_name, action.name.span());

        Some(quote::quote! {
            {
                let ctx = ctx.clone();
                cinderblock_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    route = #route_path,
                    "registering JSON API destroy endpoint"
                );
                router = router.route(
                    #route_path,
                    cinderblock_json_api::axum::routing::delete(
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
                                        (
                                            cinderblock_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            err.to_string(),
                                        )
                                            .into_response()
                                    }
                                }
                            }
                        },
                    ),
                );
            }
        })
    });

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
                quote::quote! {
                    .property(
                        #field_name,
                        <#field_type as cinderblock_json_api::FieldSchema>::field_schema(),
                    )
                    .required(#field_name)
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
        // For each enabled action, generate a `PartialSchema` + `ToSchema`
        // impl for the corresponding input struct. The field list mirrors
        // exactly what `cinderblock-core-macros` generates.
        let input_schema_impls: Vec<_> = resource
            .actions
            .iter()
            .filter_map(|action| {
                let action_name_str = action.name.to_string();
                let action_type_name = convert_case::ccase!(pascal, &action_name_str);
                let input_type =
                    Ident::new(&format!("{action_type_name}Input"), action.name.span());
                let input_type_str = format!("{action_type_name}Input");

                let accept = match &action.kind {
                    ResourceActionInputKind::Create { accept }
                        if config.should_create(&action_name_str) =>
                    {
                        accept
                    }
                    ResourceActionInputKind::Update(update)
                        if config.should_update(&action_name_str) =>
                    {
                        &update.accept
                    }
                    _ => return None,
                };

                let fields = input_fields_for_accept(&resource.attributes, accept);

                let properties: Vec<_> = fields
                    .iter()
                    .map(|(name, ty)| {
                        let name_str = name.to_string();
                        quote::quote! {
                            .property(
                                #name_str,
                                <#ty as cinderblock_json_api::FieldSchema>::field_schema(),
                            )
                            .required(#name_str)
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
        //   - Path items with operations for each enabled endpoint
        //   - Request/response body schemas referencing the components
        //   - Tags based on the resource struct name
        //   - Operation IDs like `list_ticket`, `create_ticket_open`
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
                let action_name_str = action.name.to_string();
                let action_type_name = convert_case::ccase!(pascal, &action_name_str);
                let input_type =
                    Ident::new(&format!("{action_type_name}Input"), action.name.span());
                let input_type_str = format!("{action_type_name}Input");

                let is_enabled = match &action.kind {
                    ResourceActionInputKind::Create { .. } => {
                        config.should_create(&action_name_str)
                    }
                    ResourceActionInputKind::Update(_) => config.should_update(&action_name_str),
                    ResourceActionInputKind::Destroy | ResourceActionInputKind::Read(_) => {
                        return None;
                    }
                };

                if !is_enabled {
                    return None;
                }

                Some(quote::quote! {
                    .schema(
                        #input_type_str,
                        <#input_type as cinderblock_json_api::utoipa::PartialSchema>::schema(),
                    )
                })
            })
            .collect();

        // Path items for each enabled endpoint.
        let ident_kebab = convert_case::ccase!(kebab, ident.to_string());

        // # Read path items
        //
        // GET /resource/action-name → returns filtered resources as
        // `Response<Vec<Resource>>`
        //
        // When the read action declares arguments, we add OpenAPI query
        // parameter definitions for each argument. Optional arguments
        // (those with `Option<T>` types) are marked as not required.
        let read_path_items: Vec<_> = resource
            .actions
            .iter()
            .filter_map(|action| {
                let action_read = match &action.kind {
                    ResourceActionInputKind::Read(action_read) => action_read,
                    _ => return None,
                };

                let action_name_str = action.name.to_string();
                if !config.should_read(&action_name_str) {
                    return None;
                }

                let action_path_name = convert_case::ccase!(kebab, &action_name_str);
                let route_path = format!("{}/{}", base_path, action_path_name);
                let operation_id = format!("read-{}-{}", ident_kebab, action_path_name);

                // Generate query parameter definitions for each declared
                // argument. For `Option<T>` types, extract the inner `T`
                // for the schema and mark as not required.
                let query_params: Vec<_> = action_read.arguments.iter().map(|arg| {
                    let arg_name_str = arg.name.to_string();
                    let arg_name_kebab = convert_case::ccase!(kebab, &arg_name_str);
                    let is_optional = is_option_type(&arg.ty);

                    let schema_type = if is_optional {
                        // Extract inner T from Option<T>
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
                                .name(#arg_name_kebab)
                                .parameter_in(cinderblock_json_api::utoipa::openapi::path::ParameterIn::Query)
                                .required(#required_value)
                                .schema(Some(<#schema_type as cinderblock_json_api::FieldSchema>::field_schema()))
                                .build(),
                        )
                    }
                }).collect();

                Some(quote::quote! {
                    .path(
                        #route_path,
                        cinderblock_json_api::utoipa::openapi::PathItem::new(
                            cinderblock_json_api::utoipa::openapi::path::HttpMethod::Get,
                            cinderblock_json_api::utoipa::openapi::path::OperationBuilder::new()
                                .operation_id(Some(#operation_id))
                                .tag(#ident_str)
                                .summary(Some(format!("Read {} via {}", #ident_str, #action_name_str)))
                                #(#query_params)*
                                .response(
                                    "200",
                                    cinderblock_json_api::utoipa::openapi::ResponseBuilder::new()
                                        .description(format!("Filtered list of {}s", #ident_str))
                                        .content(
                                            "application/json",
                                            cinderblock_json_api::utoipa::openapi::ContentBuilder::new()
                                                .schema(Some(
                                                    // Response<Vec<Resource>>
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
                                                        .required("data"),
                                                ))
                                                .build(),
                                        )
                                        .build(),
                                )
                                .build(),
                        ),
                    )
                })
            })
            .collect();

        let create_path_items: Vec<_> = resource
            .actions
            .iter()
            .filter_map(|action| {
                let accept = match &action.kind {
                    ResourceActionInputKind::Create { accept } => accept,
                    _ => return None,
                };

                let action_name_str = action.name.to_string();
                if !config.should_create(&action_name_str) {
                    return None;
                }

                let action_path_name = convert_case::ccase!(kebab, &action_name_str);
                let route_path = format!("{}/{}", base_path, action_path_name);
                let action_type_name = convert_case::ccase!(pascal, &action_name_str);
                let input_type =
                    Ident::new(&format!("{action_type_name}Input"), action.name.span());
                let operation_id = format!("create-{}-{}", ident_kebab, action_path_name);

                // Determine if the request body should be required (i.e., the
                // input struct has at least one field).
                let fields = input_fields_for_accept(&resource.attributes, accept);
                let body_required = !fields.is_empty();

                Some(quote::quote! {
                    .path(
                        #route_path,
                        cinderblock_json_api::utoipa::openapi::PathItem::new(
                            cinderblock_json_api::utoipa::openapi::path::HttpMethod::Post,
                            cinderblock_json_api::utoipa::openapi::path::OperationBuilder::new()
                                .operation_id(Some(#operation_id))
                                .tag(#ident_str)
                                .summary(Some(format!("Create {} via {}", #ident_str, #action_name_str)))
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
                })
            })
            .collect();

        let update_path_items: Vec<_> = resource
            .actions
            .iter()
            .filter_map(|action| {
                let update = match &action.kind {
                    ResourceActionInputKind::Update(update) => update,
                    _ => return None,
                };

                let action_name_str = action.name.to_string();
                if !config.should_update(&action_name_str) {
                    return None;
                }

                let action_path_name = convert_case::ccase!(kebab, &action_name_str);
                let route_path = format!("{}/{{primary_key}}/{}", base_path, action_path_name);
                let action_type_name = convert_case::ccase!(pascal, &action_name_str);
                let input_type =
                    Ident::new(&format!("{action_type_name}Input"), action.name.span());
                let operation_id = format!("update-{}-{}", ident_kebab, action_path_name);

                let fields = input_fields_for_accept(&resource.attributes, &update.accept);
                let body_required = !fields.is_empty();

                // Find the primary key type for the path parameter schema.
                let pk_type = resource
                    .attributes
                    .iter()
                    .find(|a| a.primary_key.value())
                    .map(|a| &a.ty);

                let pk_parameter = pk_type.map(|ty| {
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
                });

                Some(quote::quote! {
                    .path(
                        #route_path,
                        cinderblock_json_api::utoipa::openapi::PathItem::new(
                            cinderblock_json_api::utoipa::openapi::path::HttpMethod::Patch,
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
                })
            })
            .collect();

        // # Destroy path items
        //
        // DELETE /resource/{primary_key}/action → 204 No Content
        //
        // Destroy endpoints have a path parameter for the primary key but
        // no request body and no response body.
        let destroy_path_items: Vec<_> = resource
            .actions
            .iter()
            .filter_map(|action| {
                match &action.kind {
                    ResourceActionInputKind::Destroy => {}
                    _ => return None,
                }

                let action_name_str = action.name.to_string();
                if !config.should_destroy(&action_name_str) {
                    return None;
                }

                let action_path_name = convert_case::ccase!(kebab, &action_name_str);
                let route_path = format!("{}/{{primary_key}}/{}", base_path, action_path_name);
                let operation_id = format!("destroy-{}-{}", ident_kebab, action_path_name);

                // Find the primary key type for the path parameter schema.
                let pk_type = resource
                    .attributes
                    .iter()
                    .find(|a| a.primary_key.value())
                    .map(|a| &a.ty);

                let pk_parameter = pk_type.map(|ty| {
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
                });

                Some(quote::quote! {
                    .path(
                        #route_path,
                        cinderblock_json_api::utoipa::openapi::PathItem::new(
                            cinderblock_json_api::utoipa::openapi::path::HttpMethod::Delete,
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
                })
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
                            #(#read_path_items)*
                            #(#create_path_items)*
                            #(#update_path_items)*
                            #(#destroy_path_items)*
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

            #(#read_routes)*
            #(#create_routes)*
            #(#update_routes)*
            #(#destroy_routes)*

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
