// # Cinderblock Extension API
//
// Shared parser crate providing AST types and `syn::Parse` implementations for
// the `resource!` DSL. Extension macro authors depend on this crate to parse
// the forwarded resource tokens that `resource!` sends to each extension's
// `__resource_extension!` proc macro.
//
// The canonical flow is:
//   1. `resource!` parses the full DSL (using these types)
//   2. For each extension, `resource!` re-emits the raw DSL tokens plus
//      `config = { <extension-specific tokens> }` into a call to the
//      extension's proc macro
//   3. The extension proc macro parses the forwarded tokens using
//      `ExtensionMacroInput<C>`, where `C: Parse` is the extension's own
//      config type

use syn::{
    braced, bracketed, parse::Parse, punctuated::Punctuated, ExprClosure, Ident, LitBool, Token,
    Type,
};

// ---------------------------------------------------------------------------
// # Resource DSL AST Types
// ---------------------------------------------------------------------------

/// Top-level input parsed from the `resource!` macro invocation.
///
/// Represents the full DSL including name, attributes, actions, an
/// optional data layer path, and an optional extensions block.
#[derive(Debug)]
pub struct ResourceMacroInput {
    pub name: Vec<Ident>,
    pub data_layer: Option<syn::Path>,
    pub attributes: Vec<ResourceAttributeInput>,
    pub actions: Vec<ResourceActionInput>,
    pub extensions: Vec<ExtensionDecl>,
}

/// A single attribute declaration inside the `attributes { ... }` block.
///
/// Supports an optional options sub-block for `primary_key`, `generated`,
/// `writable`, and `default` settings.
#[derive(Debug)]
pub struct ResourceAttributeInput {
    pub name: Ident,
    pub ty: Type,
    pub primary_key: LitBool,
    pub generated: LitBool,
    pub writable: LitBool,
    pub default: Option<ExprClosure>,
}

impl ResourceAttributeInput {
    /// Generates a `name: Type` token stream suitable for struct field
    /// definitions or function parameters.
    pub fn to_field_definition(&self) -> proc_macro2::TokenStream {
        let name = self.name.clone();
        let ty = self.ty.clone();

        quote::quote! {
            #name: #ty
        }
    }

    /// Generates a `name: <default_expr>` token stream for fields that aren't
    /// provided by the user in a create action — either calls the user's
    /// `default` closure or falls back to `Default::default()`.
    pub fn to_default(&self) -> proc_macro2::TokenStream {
        let name = self.name.clone();

        let default = self.default.clone().map_or_else(
            || quote::quote! { ::std::default::Default::default() },
            |f| {
                quote::quote! {
                    {
                        let create = #f;
                        create()
                    }
                }
            },
        );

        quote::quote! {
            #name: #default
        }
    }
}

/// A single action declaration (e.g., `create open;` or `update close { ... };`).
#[derive(Debug)]
pub struct ResourceActionInput {
    pub kind: ResourceActionInputKind,
    pub name: Ident,
}

/// The kind-specific payload of an action — create, update, or destroy.
#[derive(Debug)]
pub enum ResourceActionInputKind {
    Create {
        accept: Accept,
    },
    Update(ActionUpdate),
    /// Destroy takes no input — the primary key is provided via the URL path.
    Destroy,
}

/// Body of an `update` action: which fields to accept and what change
/// closures to run.
#[derive(Debug)]
pub struct ActionUpdate {
    pub accept: Accept,
    pub changes: Vec<UpdateChange>,
}

/// Controls which writable attributes an action accepts as input.
///
/// `Default` means all writable attributes; `Only(vec)` restricts to the
/// listed fields.
#[derive(Debug)]
pub enum Accept {
    Default,
    Only(Vec<Ident>),
}

/// A mutation closure attached to an update action.
#[derive(Debug)]
pub enum UpdateChange {
    // TODO: support `change` (by-value) variant once needed
    Change(ExprClosure),
    ChangeRef(ExprClosure),
}

// ---------------------------------------------------------------------------
// # Extension DSL Types
// ---------------------------------------------------------------------------

/// A single extension declaration inside the `extensions { ... }` block.
///
/// Captures the extension's module path and its raw config tokens so that
/// `resource!` can forward them to the extension's proc macro.
///
/// DSL syntax:
/// ```text
/// extensions {
///     cinderblock_json_api {
///         list = true;
///     };
/// }
/// ```
#[derive(Debug)]
pub struct ExtensionDecl {
    /// The module path of the extension (e.g., `cinderblock_json_api`).
    pub path: syn::Path,
    /// Raw token stream from inside the extension's config braces.
    pub config_tokens: proc_macro2::TokenStream,
}

// ---------------------------------------------------------------------------
// # Generic Extension Macro Input
// ---------------------------------------------------------------------------

/// Input type for extension proc macros that receive forwarded tokens from
/// `resource!`.
///
/// The `resource!` macro emits calls like:
/// ```text
/// <extension>::__resource_extension! {
///     {
///         name = Helpdesk.Support.Ticket;
///         attributes { ... }
///         actions { ... }
///         extensions { ... }
///     }
///     config = { list = true; }
/// }
/// ```
///
/// The raw resource DSL tokens are forwarded verbatim inside a braced group
/// to avoid a parse-then-reconstruct roundtrip. Extension authors define a
/// config struct implementing `syn::Parse`, then parse the forwarded tokens
/// as `ExtensionMacroInput<MyConfig>`.
#[derive(Debug)]
pub struct ExtensionMacroInput<C: Parse> {
    pub resource: ResourceMacroInput,
    pub config: C,
}

// ---------------------------------------------------------------------------
// # Parsing Helpers
// ---------------------------------------------------------------------------

/// Parses an attribute of the form `<ident> = <value>;` and returns the
/// key identifier and the parsed value.
///
/// This is a convenience helper for the common DSL pattern where a
/// keyword is followed by an equals sign, a value, and a semicolon:
///
/// ```text
/// list = true;
/// openapi = false;
/// ```
///
/// The value type `V` must implement `syn::Parse`.
pub fn parse_attribute<V: Parse>(input: syn::parse::ParseStream) -> syn::Result<(Ident, V)> {
    let key: Ident = input.parse()?;
    let _: Token![=] = input.parse()?;
    let value: V = input.parse()?;
    let _: Token![;] = input.parse()?;
    Ok((key, value))
}

// ---------------------------------------------------------------------------
// # Parse Implementations
// ---------------------------------------------------------------------------

impl Parse for ResourceActionInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let kind: Ident = input.parse()?;
        let name: Ident = input.parse()?;

        let kind = match kind.to_string().as_str() {
            "create" => {
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;

                    ResourceActionInputKind::Create {
                        accept: Accept::Default,
                    }
                } else {
                    let content;
                    braced!(content in input);

                    let mut accept = Accept::Default;

                    while !content.is_empty() {
                        let key: Ident = content.parse()?;

                        match key.to_string().as_str() {
                            "accept" => {
                                let accept_content;
                                bracketed!(accept_content in content);

                                accept = Accept::Only(
                                    accept_content
                                        .parse_terminated(Ident::parse, Token![,])?
                                        .into_iter()
                                        .collect(),
                                );

                                let _: Token![;] = content.parse()?;
                            }
                            got => {
                                return Err(syn::Error::new(
                                    key.span(),
                                    format!("Unexpected create keyword, got `{got}`"),
                                ));
                            }
                        }
                    }

                    // Consume optional trailing semicolon after the closing brace,
                    // allowing both `create open { ... }` and `create open { ... };`.
                    if input.peek(Token![;]) {
                        let _: Token![;] = input.parse()?;
                    }

                    ResourceActionInputKind::Create { accept }
                }
            }
            "destroy" => {
                // Destroy actions are simple — just `destroy action_name;`
                // with no body. The primary key comes from the URL path at
                // the HTTP layer.
                let _: Token![;] = input.parse()?;

                ResourceActionInputKind::Destroy
            }
            "update" => {
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;

                    ResourceActionInputKind::Update(ActionUpdate {
                        accept: Accept::Default,
                        changes: vec![],
                    })
                } else {
                    let content;
                    braced!(content in input);

                    let mut action = ActionUpdate {
                        accept: Accept::Default,
                        changes: vec![],
                    };

                    while !content.is_empty() {
                        let key: Ident = content.parse()?;

                        match key.to_string().as_str() {
                            "accept" => {
                                let accept_content;
                                bracketed!(accept_content in content);

                                action.accept = Accept::Only(
                                    accept_content
                                        .parse_terminated(Ident::parse, Token![,])?
                                        .into_iter()
                                        .collect(),
                                );

                                let _: Token![;] = content.parse()?;
                            }
                            "change_ref" => {
                                let closure: ExprClosure = content.parse()?;
                                action.changes.push(UpdateChange::ChangeRef(closure));
                                let _: Token![;] = content.parse()?;
                            }
                            got => {
                                return Err(syn::Error::new(
                                    key.span(),
                                    format!("Unexpected update keyword, got `{got}`"),
                                ));
                            }
                        }
                    }

                    // Consume optional trailing semicolon after the closing brace,
                    // allowing both `update close { ... }` and `update close { ... };`.
                    if input.peek(Token![;]) {
                        let _: Token![;] = input.parse()?;
                    }

                    ResourceActionInputKind::Update(action)
                }
            }
            got => {
                return Err(syn::Error::new(
                    kind.span(),
                    format!("Unexpected action kind, got `{got}`"),
                ));
            }
        };

        Ok(ResourceActionInput { kind, name })
    }
}

impl Parse for ResourceMacroInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // # Name segment
        //
        // Parses `name = Helpdesk.Support.Ticket;` — a dotted identifier list.
        let _: Ident = input.parse()?; // `name`
        let _: Token![=] = input.parse()?;

        let name = Punctuated::<Ident, Token![.]>::parse_separated_nonempty(input)?
            .into_pairs()
            .map(|v| v.into_value())
            .collect::<Vec<_>>();

        let _: Token![;] = input.parse()?;

        // # Data layer (optional)
        //
        // Parses `data_layer = some::path::SqliteDataLayer;` if present.
        // When omitted, the `resource!` macro defaults to `InMemoryDataLayer`.
        let data_layer = {
            let fork = input.fork();
            if let Ok(ident) = fork.parse::<Ident>() {
                if ident == "data_layer" {
                    // Consume from the real stream now that we've confirmed
                    // the keyword.
                    let _: Ident = input.parse()?;
                    let _: Token![=] = input.parse()?;
                    let path: syn::Path = input.parse()?;
                    let _: Token![;] = input.parse()?;
                    Some(path)
                } else {
                    None
                }
            } else {
                None
            }
        };

        // # Attributes block
        //
        // Parses `attributes { <attr_declarations> }`.
        let _: Ident = input.parse()?; // `attributes`

        let content;
        braced!(content in input);

        let mut attributes = vec![];

        while !content.is_empty() {
            let name: Ident = content.parse()?;

            let mut base = ResourceAttributeInput {
                ty: content.parse()?,
                primary_key: LitBool::new(false, name.span()),
                generated: LitBool::new(false, name.span()),
                writable: LitBool::new(true, name.span()),
                default: None,
                name,
            };

            if content.peek(Token![;]) {
                let _: Token![;] = content.parse()?;
                attributes.push(base);
                continue;
            }

            let attribute_content;
            braced!(attribute_content in content);

            while !attribute_content.is_empty() {
                let name: Ident = attribute_content.parse()?;

                match name.to_string().as_str() {
                    "primary_key" => base.primary_key = attribute_content.parse()?,
                    "generated" => base.generated = attribute_content.parse()?,
                    "writable" => base.writable = attribute_content.parse()?,
                    "default" => base.default = Some(attribute_content.parse()?),
                    got => Err(syn::Error::new(
                        name.span(),
                        format!("Unexpected attribute key, got `{got}`"),
                    ))?,
                }

                let _: Token![;] = attribute_content.parse()?;
            }

            attributes.push(base);

            if content.peek(Token![;]) {
                let _: Token![;] = content.parse()?;
            }
        }

        // # Actions block (optional)
        //
        // Parses `actions { <action_declarations> }` if present.
        let mut actions = vec![];
        let mut extensions = vec![];

        // # Optional trailing sections
        //
        // After attributes, the DSL supports `actions` and `extensions` blocks
        // in any order. We peek at the next identifier and break out of the
        // loop if it's not a recognized section keyword — this is important
        // because `ExtensionMacroInput` appends a `config = { ... }` block
        // after the resource tokens, and we must leave that unconsumed.
        while input.peek(Ident) {
            let section: Ident = input.fork().parse()?;

            match section.to_string().as_str() {
                "actions" => {
                    let _: Ident = input.parse()?; // consume `actions`

                    let content;
                    braced!(content in input);

                    while !content.is_empty() {
                        actions.push(content.parse()?);
                    }
                }
                "extensions" => {
                    let _: Ident = input.parse()?; // consume `extensions`

                    let content;
                    braced!(content in input);

                    while !content.is_empty() {
                        let path: syn::Path = content.parse()?;

                        let config_content;
                        braced!(config_content in content);

                        let config_tokens: proc_macro2::TokenStream = config_content.parse()?;

                        extensions.push(ExtensionDecl {
                            path,
                            config_tokens,
                        });

                        // Optional trailing semicolon after the extension block
                        if content.peek(Token![;]) {
                            let _: Token![;] = content.parse()?;
                        }
                    }
                }
                // Unknown section keyword — stop parsing and leave the
                // remaining tokens for the caller (e.g., `config = { ... }`
                // used by `ExtensionMacroInput`).
                _ => break,
            }
        }

        Ok(ResourceMacroInput {
            name,
            data_layer,
            attributes,
            actions,
            extensions,
        })
    }
}

impl<C: Parse> Parse for ExtensionMacroInput<C> {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Parse the braced group containing the raw resource DSL tokens.
        let resource_content;
        braced!(resource_content in input);
        let resource: ResourceMacroInput = resource_content.parse()?;

        // Parse the `config = { ... }` block appended by `resource!`.
        let config_keyword: Ident = input.parse()?;
        if config_keyword != "config" {
            return Err(syn::Error::new(
                config_keyword.span(),
                format!("expected `config`, got `{config_keyword}`"),
            ));
        }
        let _: Token![=] = input.parse()?;

        let config_content;
        braced!(config_content in input);
        let config: C = config_content.parse()?;

        Ok(ExtensionMacroInput { resource, config })
    }
}

// ---------------------------------------------------------------------------
// # Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use assert2::{assert, check};
    use quote::quote;

    fn parse_resource(tokens: proc_macro2::TokenStream) -> ResourceMacroInput {
        let result = syn::parse2::<ResourceMacroInput>(tokens);
        assert!(let Ok(input) = result);
        input
    }

    #[test]
    fn minimal_resource_with_one_simple_attribute() {
        let input = parse_resource(quote! {
            name = Foo;

            attributes {
                id String;
            }
        });

        check!(input.name.len() == 1);
        check!(input.name[0] == "Foo");

        check!(input.attributes.len() == 1);
        let attr = &input.attributes[0];
        check!(attr.name == "id");
        check!(!attr.primary_key.value());
        check!(!attr.generated.value());
        check!(attr.writable.value());
        check!(attr.default.is_none());

        check!(input.actions.is_empty());
        check!(input.extensions.is_empty());
        check!(input.data_layer.is_none());
    }

    #[test]
    fn data_layer_keyword_parses_path() {
        let input = parse_resource(quote! {
            name = Ticket;
            data_layer = cinderblock_sqlx::sqlite::SqliteDataLayer;

            attributes {
                id String;
            }
        });

        assert!(let Some(path) = &input.data_layer);
        let path_str = quote::quote! { #path }.to_string();
        check!(path_str.contains("SqliteDataLayer"));
    }

    #[test]
    fn data_layer_keyword_is_optional() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
            }
        });

        check!(input.data_layer.is_none());
    }

    #[test]
    fn dotted_name_parses_into_multiple_segments() {
        let input = parse_resource(quote! {
            name = Helpdesk.Support.Ticket;

            attributes {
                id String;
            }
        });

        check!(input.name.len() == 3);
        check!(input.name[0] == "Helpdesk");
        check!(input.name[1] == "Support");
        check!(input.name[2] == "Ticket");
    }

    #[test]
    fn attribute_with_options_block() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                ticket_id Uuid {
                    primary_key true;
                    writable false;
                    default || uuid::Uuid::new_v4();
                }
            }
        });

        check!(input.attributes.len() == 1);
        let attr = &input.attributes[0];
        check!(attr.name == "ticket_id");
        check!(attr.primary_key.value());
        check!(!attr.writable.value());
        check!(attr.default.is_some());
    }

    #[test]
    fn attribute_with_generated_flag() {
        let input = parse_resource(quote! {
            name = Item;

            attributes {
                item_id Uuid {
                    primary_key true;
                    generated true;
                }
            }
        });

        let attr = &input.attributes[0];
        check!(attr.generated.value());
        check!(attr.primary_key.value());
        check!(attr.writable.value());
    }

    #[test]
    fn multiple_attributes_mixed_simple_and_complex() {
        let input = parse_resource(quote! {
            name = Order;

            attributes {
                order_id Uuid {
                    primary_key true;
                    writable false;
                }
                item_name String;
                quantity u32;
            }
        });

        check!(input.attributes.len() == 3);

        check!(input.attributes[0].name == "order_id");
        check!(input.attributes[0].primary_key.value());
        check!(!input.attributes[0].writable.value());

        check!(input.attributes[1].name == "item_name");
        check!(!input.attributes[1].primary_key.value());
        check!(input.attributes[1].writable.value());

        check!(input.attributes[2].name == "quantity");
        check!(!input.attributes[2].primary_key.value());
        check!(input.attributes[2].writable.value());
    }

    #[test]
    fn actions_block_with_simple_create() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
            }

            actions {
                create open;
            }
        });

        check!(input.actions.len() == 1);
        check!(input.actions[0].name == "open");
        check!(let ResourceActionInputKind::Create { accept: Accept::Default } = &input.actions[0].kind);
    }

    #[test]
    fn action_with_accept_list() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
            }

            actions {
                create assign {
                    accept [subject];
                };
            }
        });

        check!(input.actions.len() == 1);
        check!(input.actions[0].name == "assign");
        assert!(let ResourceActionInputKind::Create { accept: Accept::Only(idents) } = &input.actions[0].kind);
        check!(idents.len() == 1);
        check!(idents[0] == "subject");
    }

    #[test]
    fn no_actions_block_omitted() {
        let input = parse_resource(quote! {
            name = Simple;

            attributes {
                id u64;
            }
        });

        check!(input.actions.is_empty());
    }

    #[test]
    fn full_helpdesk_example() {
        let input = parse_resource(quote! {
            name = Helpdesk.Support.Ticket;

            attributes {
                ticket_id Uuid {
                    primary_key true;
                    writable false;
                    default || uuid::Uuid::new_v4();
                }

                subject String;

                status TicketStatus;
            }

            actions {
                create open;

                create assign {
                    accept [subject];
                };
            }
        });

        check!(input.name.len() == 3);
        check!(input.name[0] == "Helpdesk");
        check!(input.name[1] == "Support");
        check!(input.name[2] == "Ticket");

        check!(input.attributes.len() == 3);

        let ticket_id = &input.attributes[0];
        check!(ticket_id.name == "ticket_id");
        check!(ticket_id.primary_key.value());
        check!(!ticket_id.writable.value());
        check!(ticket_id.default.is_some());

        let subject = &input.attributes[1];
        check!(subject.name == "subject");
        check!(!subject.primary_key.value());
        check!(subject.writable.value());
        check!(subject.default.is_none());

        let status = &input.attributes[2];
        check!(status.name == "status");
        check!(!status.primary_key.value());
        check!(status.writable.value());

        check!(input.actions.len() == 2);

        check!(input.actions[0].name == "open");
        check!(let ResourceActionInputKind::Create { accept: Accept::Default } = &input.actions[0].kind);

        check!(input.actions[1].name == "assign");
        assert!(let ResourceActionInputKind::Create { accept: Accept::Only(idents) } = &input.actions[1].kind);
        check!(idents.len() == 1);
        check!(idents[0] == "subject");
    }

    #[test]
    fn parse_simple_create_action() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            create open;
        }));

        check!(action.name == "open");
        check!(let ResourceActionInputKind::Create { accept: Accept::Default } = action.kind);
    }

    #[test]
    fn parse_create_action_with_multiple_accept_idents() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            create bulk_insert {
                accept [name, email, age];
            }
        }));

        check!(action.name == "bulk_insert");
        assert!(let ResourceActionInputKind::Create { accept: Accept::Only(idents) } = action.kind);
        let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
        check!(names == vec!["name", "email", "age"]);
    }

    #[test]
    fn parse_simple_destroy_action() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            destroy foo;
        }));

        check!(action.name == "foo");
        check!(let ResourceActionInputKind::Destroy = action.kind);
    }

    #[test]
    fn unknown_action_kind_produces_error() {
        let result = syn::parse2::<ResourceActionInput>(quote! {
            frobnicate foo;
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("Unexpected action kind"));
        check!(msg.contains("frobnicate"));
    }

    #[test]
    fn unknown_attribute_option_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Thing;

            attributes {
                id String {
                    bogus true;
                }
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("Unexpected attribute key"));
        check!(msg.contains("bogus"));
    }

    #[test]
    fn missing_semicolon_after_name_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Foo

            attributes {
                id String;
            }
        });

        check!(let Err(_) = result);
    }

    #[test]
    fn parse_simple_update_action_with_default_accept() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            update close;
        }));

        check!(action.name == "close");
        assert!(let ResourceActionInputKind::Update(update) = &action.kind);
        check!(let Accept::Default = update.accept);
        check!(update.changes.is_empty());
    }

    #[test]
    fn parse_update_action_with_accept_and_change_ref() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            update close {
                accept [];
                change_ref |resource| {
                    resource.status = TicketStatus::Closed;
                };
            }
        }));

        check!(action.name == "close");
        assert!(let ResourceActionInputKind::Update(update) = &action.kind);
        assert!(let Accept::Only(idents) = &update.accept);
        check!(idents.is_empty());
        check!(update.changes.len() == 1);
        check!(let UpdateChange::ChangeRef(_) = &update.changes[0]);
    }

    #[test]
    fn parse_update_action_with_accept_fields() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            update reassign {
                accept [subject, status];
            }
        }));

        check!(action.name == "reassign");
        assert!(let ResourceActionInputKind::Update(update) = &action.kind);
        assert!(let Accept::Only(idents) = &update.accept);
        let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
        check!(names == vec!["subject", "status"]);
        check!(update.changes.is_empty());
    }

    #[test]
    fn resource_with_destroy_action() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
            }

            actions {
                create open;
                destroy remove;
            }
        });

        check!(input.actions.len() == 2);
        check!(input.actions[1].name == "remove");
        check!(let ResourceActionInputKind::Destroy = input.actions[1].kind);
    }

    // -----------------------------------------------------------------------
    // parse_attribute tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_attribute_bool() {
        let tokens = quote! { enabled = true; };
        assert!(let Ok((key, value)) = syn::parse::Parser::parse2(parse_attribute::<LitBool>, tokens));

        check!(key == "enabled");
        check!(value.value());
    }

    #[test]
    fn parse_attribute_missing_semicolon() {
        let tokens = quote! { enabled = true };
        let result = syn::parse::Parser::parse2(parse_attribute::<LitBool>, tokens);

        check!(let Err(_) = result);
    }

    #[test]
    fn parse_attribute_missing_equals() {
        let tokens = quote! { enabled true; };
        let result = syn::parse::Parser::parse2(parse_attribute::<LitBool>, tokens);

        check!(let Err(_) = result);
    }

    // -----------------------------------------------------------------------
    // Extension-specific tests
    // -----------------------------------------------------------------------

    #[test]
    fn resource_with_extensions_block() {
        let input = parse_resource(quote! {
            name = Helpdesk.Support.Ticket;

            attributes {
                id String;
            }

            extensions {
                cinderblock_json_api {
                    list = true;
                };
            }
        });

        check!(input.extensions.len() == 1);
        assert!(let Some(last_segment) = input.extensions[0].path.segments.last());
        check!(last_segment.ident == "cinderblock_json_api");
        check!(!input.extensions[0].config_tokens.is_empty());
    }

    #[test]
    fn resource_with_actions_and_extensions() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
            }

            actions {
                create open;
            }

            extensions {
                cinderblock_json_api {
                    list = true;
                };
            }
        });

        check!(input.actions.len() == 1);
        check!(input.extensions.len() == 1);
    }

    #[test]
    fn extensions_block_without_actions() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
            }

            extensions {
                cinderblock_json_api {
                    list = true;
                };
            }
        });

        check!(input.actions.is_empty());
        check!(input.extensions.len() == 1);
    }

    #[test]
    fn multiple_extensions() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
            }

            extensions {
                cinderblock_json_api {
                    list = true;
                };
                cinderblock_graphql {
                    queries = true;
                };
            }
        });

        check!(input.extensions.len() == 2);
    }

    /// Verifies that `ExtensionMacroInput<C>` correctly parses forwarded
    /// resource tokens followed by a `config = { ... }` block.
    #[test]
    fn extension_macro_input_parses_forwarded_tokens() {
        /// Minimal config struct for testing.
        struct TestConfig {
            list: bool,
        }

        impl Parse for TestConfig {
            fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
                let (key, value) = parse_attribute::<LitBool>(input)?;
                check!(key == "list");
                Ok(TestConfig {
                    list: value.value(),
                })
            }
        }

        assert!(let Ok(input) = syn::parse2::<ExtensionMacroInput<TestConfig>>(quote! {
            {
                name = Helpdesk.Support.Ticket;

                attributes {
                    id String;
                }

                extensions {
                    cinderblock_json_api {
                        list = true;
                    };
                }
            }

            config = {
                list = true;
            }
        }));

        check!(input.resource.name.len() == 3);
        check!(input.config.list);
    }
}
