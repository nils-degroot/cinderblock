// # JSON API Extension Proc Macro
//
// This proc macro is invoked by the `resource!` macro when a resource declares
// `ash_json_api` in its `extensions { ... }` block. It receives the full
// resource DSL tokens plus the extension-specific config, and generates:
//
//   1. A route registration function that wires up the resource's endpoints
//   2. An `inventory::submit!` call that auto-registers the endpoints so
//      `ash_json_api::router()` can discover them without manual wiring
//
// # Supported endpoints
//
// - **list**: `GET /resource_path` — returns all resources
// - **create**: `POST /resource_path/:action` — creates a resource via the
//   named action
// - **update**: `PATCH /resource_path/{primary_key}/:action` — updates a
//   resource via the named action
//
// # Config semantics
//
// An empty config block (`ash_json_api {}`) exposes everything: the list
// endpoint plus all declared create and update actions. If any of `list`,
// `create`, or `update` is explicitly specified, only those are exposed.
//
// ```text
// ash_json_api {};                          // expose everything
// ash_json_api { list = true; };            // only list
// ash_json_api { create = [open]; };        // only the "open" create action
// ash_json_api { create = [open]; list = true; };  // list + open
// ```

use ash_extension_api::{ExtensionMacroInput, ResourceActionInputKind};
use syn::{bracketed, parse::Parse, Ident, LitBool, Token};

/// Extension-specific configuration parsed from inside the `config = { ... }`
/// block.
///
/// When all fields are `None` (empty config), the extension exposes
/// everything. When any field is `Some`, only the explicitly enabled
/// endpoints are generated.
struct JsonApiConfig {
    list: Option<bool>,
    create: Option<Vec<Ident>>,
    update: Option<Vec<Ident>>,
}

impl JsonApiConfig {
    /// Returns `true` if the user explicitly configured any endpoints,
    /// meaning we should only expose what was listed.
    fn is_explicit(&self) -> bool {
        self.list.is_some() || self.create.is_some() || self.update.is_some()
    }

    fn should_list(&self) -> bool {
        if self.is_explicit() {
            self.list.unwrap_or(false)
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
}

impl Parse for JsonApiConfig {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut config = JsonApiConfig {
            list: None,
            create: None,
            update: None,
        };

        while !input.is_empty() {
            let key: Ident = input.parse()?;

            match key.to_string().as_str() {
                "list" => {
                    let _: Token![=] = input.parse()?;
                    let value: LitBool = input.parse()?;
                    config.list = Some(value.value());
                    let _: Token![;] = input.parse()?;
                }
                "create" => {
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
                got => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unexpected ash_json_api config key, got `{got}`"),
                    ));
                }
            }
        }

        Ok(config)
    }
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
    // The base route path is built by lowercasing each name segment and
    // joining with `/`. For example, `Helpdesk.Support.Ticket` becomes
    // `/helpdesk/support/ticket`.
    let base_path = format!(
        "/{}",
        resource
            .name
            .iter()
            .map(|s| s.to_string().to_lowercase())
            .collect::<Vec<_>>()
            .join("/")
    );

    // Generate a unique function name for the registration function to avoid
    // collisions when multiple resources register endpoints.
    let register_fn_name = Ident::new(
        &format!(
            "__register_json_api_{}",
            resource
                .name
                .iter()
                .map(|s| s.to_string().to_lowercase())
                .collect::<Vec<_>>()
                .join("_")
        ),
        ident.span(),
    );

    // # List endpoint
    //
    // GET /helpdesk/support/ticket → returns all resources
    let list_route = if config.should_list() {
        let route_path = &base_path;
        Some(quote::quote! {
            {
                let ctx = ctx.clone();
                ash_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    route = #route_path,
                    "registering JSON API list endpoint"
                );
                router = router.route(
                    #route_path,
                    ash_json_api::axum::routing::get(move || {
                        let ctx = ctx.clone();
                        async move {
                            ash_json_api::tracing::info!(
                                resource = stringify!(#ident),
                                "handling list request"
                            );

                            match ash_core::list::<#ident>(&ctx).await {
                                Ok(results) => (
                                    ash_json_api::axum::http::StatusCode::OK,
                                    ash_json_api::axum::Json(
                                        ash_json_api::Response { data: results },
                                    ),
                                )
                                    .into_response(),
                                Err(err) => {
                                    ash_json_api::tracing::error!(
                                        resource = stringify!(#ident),
                                        error = %err,
                                        "list request failed"
                                    );
                                    (
                                        ash_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                        err.to_string(),
                                    )
                                        .into_response()
                                }
                            }
                        }
                    }),
                );
            }
        })
    } else {
        None
    };

    // # Create endpoints
    //
    // POST /helpdesk/support/ticket/open → creates via the "open" action
    //
    // For each create action declared on the resource, we generate a POST
    // route. The action name is used as a path segment and converted to
    // PascalCase to reference the generated marker and input structs.
    let create_routes = resource.actions.iter().filter_map(|action| {
        match &action.kind {
            ResourceActionInputKind::Create { .. } => {}
            _ => return None,
        }

        let action_name_str = action.name.to_string();
        if !config.should_create(&action_name_str) {
            return None;
        }

        let route_path = format!("{}/{}", base_path, action_name_str);

        let action_type_name = convert_case::ccase!(pascal, &action_name_str);
        let action_type = Ident::new(&action_type_name, action.name.span());
        let input_type = Ident::new(&format!("{action_type_name}Input"), action.name.span());

        Some(quote::quote! {
            {
                let ctx = ctx.clone();
                ash_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    route = #route_path,
                    "registering JSON API create endpoint"
                );
                router = router.route(
                    #route_path,
                    ash_json_api::axum::routing::post(
                        move |ash_json_api::axum::Json(input): ash_json_api::axum::Json<#input_type>| {
                            let ctx = ctx.clone();
                            async move {
                                ash_json_api::tracing::info!(
                                    resource = stringify!(#ident),
                                    action = #action_name_str,
                                    "handling create request"
                                );

                                match ash_core::create::<#ident, #action_type>(input, &ctx).await {
                                    Ok(created) => (
                                        ash_json_api::axum::http::StatusCode::CREATED,
                                        ash_json_api::axum::Json(
                                            ash_json_api::Response { data: created },
                                        ),
                                    )
                                        .into_response(),
                                    Err(err) => {
                                        ash_json_api::tracing::error!(
                                            resource = stringify!(#ident),
                                            action = #action_name_str,
                                            error = %err,
                                            "create request failed"
                                        );
                                        (
                                            ash_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
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

        let route_path = format!("{}/{{primary_key}}/{}", base_path, action_name_str);

        let action_type_name = convert_case::ccase!(pascal, &action_name_str);
        let action_type = Ident::new(&action_type_name, action.name.span());
        let input_type = Ident::new(&format!("{action_type_name}Input"), action.name.span());

        Some(quote::quote! {
            {
                let ctx = ctx.clone();
                ash_json_api::tracing::info!(
                    resource = stringify!(#ident),
                    action = #action_name_str,
                    route = #route_path,
                    "registering JSON API update endpoint"
                );
                router = router.route(
                    #route_path,
                    ash_json_api::axum::routing::patch(
                        move |
                            ash_json_api::axum::extract::Path(primary_key): ash_json_api::axum::extract::Path<
                                <#ident as ash_core::Resource>::PrimaryKey,
                            >,
                            ash_json_api::axum::Json(input): ash_json_api::axum::Json<#input_type>,
                        | {
                            let ctx = ctx.clone();
                            async move {
                                ash_json_api::tracing::info!(
                                    resource = stringify!(#ident),
                                    action = #action_name_str,
                                    %primary_key,
                                    "handling update request"
                                );

                                match ash_core::update::<#ident, #action_type>(
                                    &primary_key,
                                    input,
                                    &ctx,
                                )
                                .await
                                {
                                    Ok(updated) => (
                                        ash_json_api::axum::http::StatusCode::OK,
                                        ash_json_api::axum::Json(
                                            ash_json_api::Response { data: updated },
                                        ),
                                    )
                                        .into_response(),
                                    Err(err) => {
                                        ash_json_api::tracing::error!(
                                            resource = stringify!(#ident),
                                            action = #action_name_str,
                                            error = %err,
                                            "update request failed"
                                        );
                                        (
                                            ash_json_api::axum::http::StatusCode::INTERNAL_SERVER_ERROR,
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

    quote::quote! {
        fn #register_fn_name(
            mut router: ash_json_api::axum::Router,
            ctx: ::std::sync::Arc<ash_core::Context>,
        ) -> ash_json_api::axum::Router {
            use ash_json_api::axum::response::IntoResponse;

            #list_route
            #(#create_routes)*
            #(#update_routes)*

            router
        }

        ash_json_api::inventory::submit! {
            ash_json_api::ResourceEndpoint {
                register: #register_fn_name,
            }
        }
    }
    .into()
}
