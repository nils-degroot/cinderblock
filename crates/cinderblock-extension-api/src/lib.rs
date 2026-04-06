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
    ExprClosure, Ident, LitBool, Path, Token, Type, braced, bracketed, parenthesized, parse::Parse,
    punctuated::Punctuated,
};

use crate::domain::resource_name::ResourceName;

pub mod domain;
pub mod util;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// # Resource DSL AST Types
// ---------------------------------------------------------------------------

/// Top-level input parsed from the `resource!` macro invocation.
///
/// Represents the full DSL including name, attributes, actions, an
/// optional data layer path, and an optional extensions block.
#[derive(Debug, Clone)]
pub struct ResourceMacroInput {
    pub name: ResourceName,
    pub data_layer: Option<Path>,
    pub attributes: Vec<ResourceAttributeInput>,
    pub relations: Vec<RelationDecl>,
    pub actions: Vec<ResourceActionInput>,
    pub extensions: Vec<ExtensionDecl>,
    pub before_create: Option<ExprClosure>,
    pub before_update: Option<ExprClosure>,
}

impl ResourceMacroInput {
    pub fn primary_keys(&self) -> impl Iterator<Item = &ResourceAttributeInput> {
        self.attributes
            .iter()
            .filter(|attr| attr.primary_key.value())
    }
}

/// A single attribute declaration inside the `attributes { ... }` block.
///
/// Supports an optional options sub-block for `primary_key`, `generated`,
/// `writable`, and `default` settings.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct ResourceActionInput {
    pub kind: ResourceActionInputKind,
    pub raw_name: Ident,
    pub action_name: Ident,
}

/// The kind-specific payload of an action — create, update, or destroy.
#[derive(Debug, Clone)]
pub enum ResourceActionInputKind {
    Read(ActionRead),
    Create(ActionCreate),
    Update(ActionUpdate),
    /// Destroy takes no input — the primary key is provided via the URL path.
    Destroy,
}

#[derive(Debug, Clone)]
pub struct ActionRead {
    pub arguments: Vec<ReadArgument>,
    pub filters: Vec<ReadFilter>,
    pub orders: Vec<OrderClause>,
    pub paged: Option<PagedConfig>,
    /// Relations to eagerly load for this read action.
    ///
    /// Parsed from `load [author, comments];` inside a read action body.
    /// Each identifier must reference a relation declared in the resource's
    /// `relations { ... }` block. When empty, only the base resource columns
    /// are returned.
    pub load: Vec<Ident>,
    /// When `true`, this read action fetches a single resource by primary key
    /// instead of returning a list.
    ///
    /// Parsed from `get;` inside a read action body. Mutually exclusive with
    /// `paged`, `filter`, `order`, and `argument`. Can be combined with `load`
    /// to eagerly load relations on a single-resource fetch.
    pub get: bool,
}

/// Configuration for paged read actions.
///
/// Parsed from `paged;` (all defaults) or `paged { default_per_page 50; max_per_page 200; };`
/// inside a read action body. When both fields are `None`, framework defaults apply.
#[derive(Debug, Clone)]
pub struct PagedConfig {
    /// Default number of items per page when the client doesn't specify `per_page`.
    /// When `None`, the framework default (`cinderblock_core::DEFAULT_PER_PAGE`) is used.
    pub default_per_page: Option<u32>,
    /// Maximum allowed `per_page` value. Client requests exceeding this are
    /// silently clamped. When `None`, defaults to `default_per_page` (or the
    /// framework default).
    pub max_per_page: Option<u32>,
}

/// A single argument declared in a read action's `argument { ... }` block.
///
/// Optionality is determined by the Rust type itself: if the user writes
/// `Option<String>`, it's optional; if they write `String`, it's required.
/// No separate keyword is needed.
#[derive(Debug, Clone)]
pub struct ReadArgument {
    pub name: Ident,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct ReadFilter {
    pub field: Ident,
    pub op: ReadFilterOperation,
    pub value: ReadFilterValue,
}

/// The right-hand side of a filter expression. Either a compile-time literal
/// expression (e.g., `false`, `42`, `TicketStatus::Open`) or a reference to a
/// runtime argument via `arg(name)`.
#[derive(Debug, Clone)]
pub enum ReadFilterValue {
    Literal(syn::Expr),
    Arg(Ident),
}

#[derive(Debug, Clone, Copy)]
pub enum ReadFilterOperation {
    Eq,
}

impl Parse for ReadFilterOperation {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.peek(Token![==]) {
            let _: Token![==] = input.parse()?;
            Ok(Self::Eq)
        } else {
            Err(syn::Error::new(
                input.span(),
                "Unexpected token, expected a filter operation",
            ))
        }
    }
}

/// The sort direction for an order clause.
#[derive(Debug, Clone, Copy)]
pub enum OrderDirection {
    Asc,
    Desc,
}

/// A single ordering clause declared in a read action's `order { ... }` block.
///
/// DSL syntax: `order { field_name desc; field_name2; };`
/// When the direction is omitted, `Asc` is used as the default.
#[derive(Debug, Clone)]
pub struct OrderClause {
    pub field: Ident,
    pub direction: OrderDirection,
}

impl Parse for ReadArgument {
    /// Parses `name: Type` — a single argument declaration.
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;
        Ok(ReadArgument { name, ty })
    }
}

/// Body of an `update` action: which fields to accept and what change
/// closures to run.
#[derive(Debug, Clone)]
pub struct ActionUpdate {
    pub accept: Accept,
    pub changes: Vec<UpdateChange>,
}

/// Controls which writable attributes an action accepts as input.
///
/// `Default` means all writable attributes; `Only(vec)` restricts to the
/// listed fields.
#[derive(Debug, Default, Clone)]
pub enum Accept {
    #[default]
    Default,
    Only(Vec<Ident>),
}

//---------------------------------------------------------------------------
// # Relation DSL Types
// ---------------------------------------------------------------------------

/// Whether a relation is a belongs-to (FK on this resource) or a has-many
/// (FK on the related resource pointing back here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    BelongsTo,
    HasMany,
}

/// A single relation declaration inside the `relations { ... }` block.
///
/// DSL syntax:
/// ```text
/// relations {
///     belongs_to author {
///         ty User;
///         source_attribute author_id;
///     };
///     has_many comments {
///         ty Comment;
///         source_attribute post_id;
///     };
/// }
/// ```
///
/// `source_attribute` is the foreign key column name. For `belongs_to`, it
/// lives on *this* resource. For `has_many`, it lives on the *related*
/// resource.
///
/// `destination_attribute` is optional and defaults to the destination
/// resource's primary key when omitted.
#[derive(Debug, Clone)]
pub struct RelationDecl {
    pub name: Ident,
    pub kind: RelationKind,
    /// The destination resource type (e.g., `User`, `Comment`).
    pub ty: Type,
    /// The foreign key attribute name.
    pub source_attribute: Ident,
    /// The attribute on the destination resource to match against.
    /// Defaults to the destination resource's primary key when `None`.
    pub destination_attribute: Option<Ident>,
}

#[derive(Debug, Default, Clone)]
pub struct ActionCreate {
    pub accept: Accept,
}

/// A mutation closure attached to an update action.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct ExtensionDecl {
    /// The module path of the extension (e.g., `cinderblock_json_api`).
    pub path: Path,
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

/// Parses the contents of an attribute options block (`{ primary_key true; writable false; ... }`)
/// and applies each setting to the given `base` attribute.
///
/// Reused by both the normal attribute path and the `uuid_primary_key` shortcut
/// override block so that the set of recognised keys stays in one place.
fn parse_attribute_options(
    attribute_content: syn::parse::ParseStream,
    base: &mut ResourceAttributeInput,
) -> syn::Result<()> {
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
    Ok(())
}

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
        let raw_name: Ident = input.parse()?;

        let kind = match kind.to_string().as_str() {
            "read" => {
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                    ResourceActionInputKind::Read(ActionRead {
                        arguments: vec![],
                        filters: vec![],
                        orders: vec![],
                        paged: None,
                        load: vec![],
                        get: false,
                    })
                } else {
                    let content;
                    braced!(content in input);

                    let mut action = ActionRead {
                        arguments: vec![],
                        filters: vec![],
                        orders: vec![],
                        paged: None,
                        load: vec![],
                        get: false,
                    };

                    while !content.is_empty() {
                        let key: Ident = content.parse()?;

                        match key.to_string().as_str() {
                            "argument" => {
                                // Parses `argument { name: Type, name2: Type };`
                                let arg_content;
                                braced!(arg_content in content);

                                let pairs =
                                    Punctuated::<ReadArgument, Token![,]>::parse_terminated(
                                        &arg_content,
                                    )?;
                                action.arguments.extend(pairs);

                                let _: Token![;] = content.parse()?;
                            }
                            "filter" => {
                                let filter_content;
                                braced!(filter_content in content);

                                let field: Ident = filter_content.parse()?;
                                let op: ReadFilterOperation = filter_content.parse()?;

                                // # Filter Value Parsing
                                //
                                // If the next token is `arg`, parse `arg(name)` as a
                                // runtime argument reference. Otherwise parse as a
                                // literal expression.
                                let value = if filter_content.peek(Ident) {
                                    let fork = filter_content.fork();
                                    let maybe_arg: Ident = fork.parse()?;
                                    if maybe_arg == "arg" {
                                        // Consume from the real stream
                                        let _: Ident = filter_content.parse()?;
                                        let paren_content;
                                        parenthesized!(paren_content in filter_content);
                                        let arg_name: Ident = paren_content.parse()?;
                                        ReadFilterValue::Arg(arg_name)
                                    } else {
                                        ReadFilterValue::Literal(filter_content.parse()?)
                                    }
                                } else {
                                    ReadFilterValue::Literal(filter_content.parse()?)
                                };

                                action.filters.push(ReadFilter { field, op, value });

                                let _: Token![;] = content.parse()?;
                            }
                            "paged" => {
                                // # Paged keyword parsing
                                //
                                // Supports two forms:
                                //   - `paged;` — use framework defaults
                                //   - `paged { default_per_page 50; max_per_page 200; };` —
                                //     per-action overrides
                                if content.peek(Token![;]) {
                                    let _: Token![;] = content.parse()?;
                                    action.paged = Some(PagedConfig {
                                        default_per_page: None,
                                        max_per_page: None,
                                    });
                                } else {
                                    let paged_content;
                                    braced!(paged_content in content);

                                    let mut paged_config = PagedConfig {
                                        default_per_page: None,
                                        max_per_page: None,
                                    };

                                    while !paged_content.is_empty() {
                                        let paged_key: Ident = paged_content.parse()?;
                                        match paged_key.to_string().as_str() {
                                            "default_per_page" => {
                                                let lit: syn::LitInt = paged_content.parse()?;
                                                paged_config.default_per_page =
                                                    Some(lit.base10_parse()?);
                                                let _: Token![;] = paged_content.parse()?;
                                            }
                                            "max_per_page" => {
                                                let lit: syn::LitInt = paged_content.parse()?;
                                                paged_config.max_per_page =
                                                    Some(lit.base10_parse()?);
                                                let _: Token![;] = paged_content.parse()?;
                                            }
                                            got => {
                                                return Err(syn::Error::new(
                                                    paged_key.span(),
                                                    format!(
                                                        "Unexpected paged config key, got `{got}`. \
                                                         Expected `default_per_page` or `max_per_page`."
                                                    ),
                                                ));
                                            }
                                        }
                                    }

                                    action.paged = Some(paged_config);

                                    let _: Token![;] = content.parse()?;
                                }
                            }
                            "load" => {
                                // # Load keyword parsing
                                //
                                // Parses `load [author, comments];` — a bracketed
                                // list of relation names to eagerly load for this
                                // read action.
                                let load_content;
                                bracketed!(load_content in content);

                                action.load = load_content
                                    .parse_terminated(Ident::parse, Token![,])?
                                    .into_iter()
                                    .collect();

                                let _: Token![;] = content.parse()?;
                            }
                            "order" => {
                                let order_content;
                                braced!(order_content in content);

                                while !order_content.is_empty() {
                                    let field: Ident = order_content.parse()?;

                                    let direction = if order_content.peek(Token![;]) {
                                        OrderDirection::Asc
                                    } else {
                                        let dir_ident: Ident = order_content.parse()?;
                                        match dir_ident.to_string().as_str() {
                                            "asc" => OrderDirection::Asc,
                                            "desc" => OrderDirection::Desc,
                                            got => {
                                                return Err(syn::Error::new(
                                                    dir_ident.span(),
                                                    format!(
                                                        "Unexpected order direction, got `{got}`. \
                                                         Expected `asc` or `desc`."
                                                    ),
                                                ));
                                            }
                                        }
                                    };

                                    let _: Token![;] = order_content.parse()?;
                                    action.orders.push(OrderClause { field, direction });
                                }

                                let _: Token![;] = content.parse()?;
                            }
                            "get" => {
                                let _: Token![;] = content.parse()?;
                                action.get = true;
                            }
                            got => {
                                return Err(syn::Error::new(
                                    key.span(),
                                    format!("Unexpected read keyword, got `{got}`"),
                                ));
                            }
                        }
                    }

                    // Validate that all `arg(name)` references in filters point
                    // to declared arguments.
                    for filter in &action.filters {
                        if let ReadFilterValue::Arg(ref arg_name) = filter.value
                            && !action.arguments.iter().any(|a| a.name == *arg_name)
                        {
                            return Err(syn::Error::new(
                                arg_name.span(),
                                format!(
                                    "Filter references undeclared argument `{arg_name}`. \
                                     Declare it in the `argument {{ ... }}` block."
                                ),
                            ));
                        }
                    }

                    if action.get {
                        macro_rules! reject_with_get {
                            ($cond:expr, $keyword:literal) => {
                                if $cond {
                                    return Err(syn::Error::new(
                                        raw_name.span(),
                                        concat!(
                                            "`get` is mutually exclusive with `",
                                            $keyword,
                                            "`"
                                        ),
                                    ));
                                }
                            };
                        }

                        reject_with_get!(action.paged.is_some(), "paged");
                        reject_with_get!(!action.filters.is_empty(), "filter");
                        reject_with_get!(!action.orders.is_empty(), "order");
                        reject_with_get!(!action.arguments.is_empty(), "argument");
                    }

                    if input.peek(Token![;]) {
                        let _: Token![;] = input.parse()?;
                    }

                    ResourceActionInputKind::Read(action)
                }
            }
            "create" => {
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                    ResourceActionInputKind::Create(ActionCreate::default())
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

                    ResourceActionInputKind::Create(ActionCreate { accept })
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

        let action_name = convert_case::ccase!(pascal, raw_name.to_string());
        let action_name = Ident::new(&action_name, raw_name.span());

        Ok(ResourceActionInput {
            kind,
            action_name,
            raw_name,
        })
    }
}

impl Parse for ResourceMacroInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // # Name segment
        //
        // Parses `name = Helpdesk.Support.Ticket;` — a dotted identifier list.
        let _: Ident = input.parse()?; // `name`
        let _: Token![=] = input.parse()?;

        let name = ResourceName::parse(input)?;

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
                    let path: Path = input.parse()?;
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

            // uuid_primary_key shortcut: expands to Uuid type with
            // primary_key=true, writable=false, default=|| uuid::Uuid::new_v4()
            let mut base = if content.peek(Ident)
                && content
                    .fork()
                    .parse::<Ident>()
                    .is_ok_and(|id| id == "uuid_primary_key")
            {
                let _: Ident = content.parse()?;

                ResourceAttributeInput {
                    ty: syn::parse_quote!(uuid::Uuid),
                    primary_key: LitBool::new(true, name.span()),
                    generated: LitBool::new(false, name.span()),
                    writable: LitBool::new(false, name.span()),
                    default: Some(syn::parse_quote!(|| uuid::Uuid::new_v4())),
                    name,
                }
            } else {
                ResourceAttributeInput {
                    ty: content.parse()?,
                    primary_key: LitBool::new(false, name.span()),
                    generated: LitBool::new(false, name.span()),
                    writable: LitBool::new(true, name.span()),
                    default: None,
                    name,
                }
            };

            if content.peek(Token![;]) {
                let _: Token![;] = content.parse()?;
                attributes.push(base);
                continue;
            }

            let attribute_content;
            braced!(attribute_content in content);
            parse_attribute_options(&attribute_content, &mut base)?;

            attributes.push(base);

            if content.peek(Token![;]) {
                let _: Token![;] = content.parse()?;
            }
        }

        // # Actions block (optional)
        //
        // Parses `actions { <action_declarations> }` if present.
        let mut actions: Vec<ResourceActionInput> = vec![];
        let mut extensions = vec![];
        let mut relations = vec![];
        let mut before_create: Option<ExprClosure> = None;
        let mut before_update: Option<ExprClosure> = None;

        // # Optional trailing sections
        //
        // After attributes, the DSL supports `relations`, `actions`, and
        // `extensions` blocks in any order. We peek at the next identifier
        // and break out of the loop if it's not a recognized section
        // keyword — this is important because `ExtensionMacroInput` appends
        // a `config = { ... }` block after the resource tokens, and we must
        // leave that unconsumed.
        while input.peek(Ident) {
            let section: Ident = input.fork().parse()?;

            match section.to_string().as_str() {
                "relations" => {
                    let _: Ident = input.parse()?; // consume `relations`

                    let content;
                    braced!(content in input);

                    // # Relation Declaration Parsing
                    //
                    // Each relation follows the pattern:
                    //   belongs_to <name> { ty <Type>; source_attribute <ident>; };
                    //   has_many <name> { ty <Type>; source_attribute <ident>; };
                    while !content.is_empty() {
                        let kind_ident: Ident = content.parse()?;
                        let kind = match kind_ident.to_string().as_str() {
                            "belongs_to" => RelationKind::BelongsTo,
                            "has_many" => RelationKind::HasMany,
                            got => {
                                return Err(syn::Error::new(
                                    kind_ident.span(),
                                    format!(
                                        "Unexpected relation kind `{got}`. \
                                         Expected `belongs_to` or `has_many`."
                                    ),
                                ));
                            }
                        };

                        let name: Ident = content.parse()?;

                        let rel_content;
                        braced!(rel_content in content);

                        let mut ty: Option<Type> = None;
                        let mut source_attribute: Option<Ident> = None;
                        let mut destination_attribute: Option<Ident> = None;

                        while !rel_content.is_empty() {
                            let rel_key: Ident = rel_content.parse()?;

                            match rel_key.to_string().as_str() {
                                "ty" => {
                                    ty = Some(rel_content.parse()?);
                                    let _: Token![;] = rel_content.parse()?;
                                }
                                "source_attribute" => {
                                    source_attribute = Some(rel_content.parse()?);
                                    let _: Token![;] = rel_content.parse()?;
                                }
                                "destination_attribute" => {
                                    destination_attribute = Some(rel_content.parse()?);
                                    let _: Token![;] = rel_content.parse()?;
                                }
                                got => {
                                    return Err(syn::Error::new(
                                        rel_key.span(),
                                        format!(
                                            "Unexpected relation key `{got}`. Expected \
                                             `ty`, `source_attribute`, or `destination_attribute`."
                                        ),
                                    ));
                                }
                            }
                        }

                        let ty = ty.ok_or_else(|| {
                            syn::Error::new(
                                name.span(),
                                format!("Relation `{name}` is missing required `ty` declaration."),
                            )
                        })?;

                        let source_attribute = source_attribute.ok_or_else(|| {
                            syn::Error::new(
                                name.span(),
                                format!(
                                    "Relation `{name}` is missing required \
                                     `source_attribute` declaration."
                                ),
                            )
                        })?;

                        relations.push(RelationDecl {
                            name,
                            kind,
                            ty,
                            source_attribute,
                            destination_attribute,
                        });

                        // Optional trailing semicolon after the relation block
                        if content.peek(Token![;]) {
                            let _: Token![;] = content.parse()?;
                        }
                    }
                }
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
                        let path: Path = content.parse()?;

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
                "before_create" => {
                    let _: Ident = input.parse()?;

                    if before_create.is_some() {
                        return Err(syn::Error::new(
                            section.span(),
                            "`before_create` can only be declared once per resource.",
                        ));
                    }

                    before_create = Some(input.parse()?);
                    let _: Token![;] = input.parse()?;
                }
                "before_update" => {
                    let _: Ident = input.parse()?;

                    if before_update.is_some() {
                        return Err(syn::Error::new(
                            section.span(),
                            "`before_update` can only be declared once per resource.",
                        ));
                    }

                    before_update = Some(input.parse()?);
                    let _: Token![;] = input.parse()?;
                }
                // Unknown section keyword — stop parsing and leave the
                // remaining tokens for the caller (e.g., `config = { ... }`
                // used by `ExtensionMacroInput`).
                _ => break,
            }
        }

        // # Load reference validation
        //
        // Verify that every `load [...]` identifier in read actions
        // references a relation declared in the `relations { ... }` block.
        for action in &actions {
            if let ResourceActionInputKind::Read(read) = &action.kind {
                for load_name in &read.load {
                    if !relations.iter().any(|r| r.name == *load_name) {
                        return Err(syn::Error::new(
                            load_name.span(),
                            format!(
                                "Read action `{}` loads undeclared relation `{load_name}`. \
                                 Declare it in the `relations {{ ... }}` block.",
                                action.raw_name,
                            ),
                        ));
                    }
                }

                for order in &read.orders {
                    if !attributes.iter().any(|a| a.name == order.field) {
                        return Err(syn::Error::new(
                            order.field.span(),
                            format!(
                                "Read action `{}` orders by undeclared attribute `{}`. \
                                 Declare it in the `attributes {{ ... }}` block.",
                                action.raw_name, order.field,
                            ),
                        ));
                    }
                }
            }
        }

        Ok(ResourceMacroInput {
            name,
            data_layer,
            attributes,
            relations,
            actions,
            extensions,
            before_create,
            before_update,
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
