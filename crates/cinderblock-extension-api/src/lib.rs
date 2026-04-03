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
    pub data_layer: Option<Path>,
    pub attributes: Vec<ResourceAttributeInput>,
    pub relations: Vec<RelationDecl>,
    pub actions: Vec<ResourceActionInput>,
    pub extensions: Vec<ExtensionDecl>,
    /// Optional lifecycle hook that runs on every create action, after the
    /// resource struct is built from input but before persistence.
    pub before_create: Option<ExprClosure>,
    /// Optional lifecycle hook that runs on every update action, after
    /// `apply_update_input` but before persistence.
    pub before_update: Option<ExprClosure>,
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
    Read(ActionRead),
    Create {
        accept: Accept,
    },
    Update(ActionUpdate),
    /// Destroy takes no input — the primary key is provided via the URL path.
    Destroy,
}

#[derive(Debug)]
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
    /// `paged`, `filter`, `order`, `argument`, and `load`.
    pub get: bool,
}

/// Configuration for paged read actions.
///
/// Parsed from `paged;` (all defaults) or `paged { default_per_page 50; max_per_page 200; };`
/// inside a read action body. When both fields are `None`, framework defaults apply.
#[derive(Debug)]
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
#[derive(Debug)]
pub struct ReadArgument {
    pub name: Ident,
    pub ty: Type,
}

#[derive(Debug)]
pub struct ReadFilter {
    pub field: Ident,
    pub op: ReadFilterOperation,
    pub value: ReadFilterValue,
}

/// The right-hand side of a filter expression. Either a compile-time literal
/// expression (e.g., `false`, `42`, `TicketStatus::Open`) or a reference to a
/// runtime argument via `arg(name)`.
#[derive(Debug)]
pub enum ReadFilterValue {
    Literal(syn::Expr),
    Arg(Ident),
}

#[derive(Debug)]
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
#[derive(Debug)]
pub enum OrderDirection {
    Asc,
    Desc,
}

/// A single ordering clause declared in a read action's `order { ... }` block.
///
/// DSL syntax: `order { field_name desc; field_name2; };`
/// When the direction is omitted, `Asc` is used as the default.
#[derive(Debug)]
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

// ---------------------------------------------------------------------------
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
#[derive(Debug)]
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
        let name: Ident = input.parse()?;

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
                                        name.span(),
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
                        reject_with_get!(!action.load.is_empty(), "load");
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
                                action.name,
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
                                action.name, order.field,
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

    // -----------------------------------------------------------------------
    // Read action argument + arg() filter tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_action_with_no_arguments_or_filters() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read all;
        }));

        check!(action.name == "all");
        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.arguments.is_empty());
        check!(read.filters.is_empty());
    }

    #[test]
    fn read_action_with_literal_filter_only() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read open_only {
                filter { archived == false };
            }
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.arguments.is_empty());
        check!(read.filters.len() == 1);
        check!(read.filters[0].field == "archived");
        check!(let ReadFilterValue::Literal(_) = &read.filters[0].value);
    }

    #[test]
    fn read_action_with_arguments_and_arg_filter() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read by_status {
                argument { status: TicketStatus };
                filter { status == arg(status) };
            }
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.arguments.len() == 1);
        check!(read.arguments[0].name == "status");
        check!(read.filters.len() == 1);
        assert!(let ReadFilterValue::Arg(arg_name) = &read.filters[0].value);
        check!(*arg_name == "status");
    }

    #[test]
    fn read_action_with_multiple_arguments() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read by_status_and_priority {
                argument { status: TicketStatus, search: Option<String> };
                filter { status == arg(status) };
                filter { archived == false };
            }
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.arguments.len() == 2);
        check!(read.arguments[0].name == "status");
        check!(read.arguments[1].name == "search");
        check!(read.filters.len() == 2);
        check!(let ReadFilterValue::Arg(_) = &read.filters[0].value);
        check!(let ReadFilterValue::Literal(_) = &read.filters[1].value);
    }

    #[test]
    fn read_action_rejects_undeclared_arg_reference() {
        let result = syn::parse2::<ResourceActionInput>(quote! {
            read broken {
                filter { status == arg(status) };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("undeclared argument"));
        check!(msg.contains("status"));
    }

    #[test]
    fn read_action_mixed_arg_and_literal_filters() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read filtered {
                argument { priority: u32 };
                filter { priority == arg(priority) };
                filter { archived == false };
            }
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.filters.len() == 2);
        assert!(let ReadFilterValue::Arg(name) = &read.filters[0].value);
        check!(*name == "priority");
        check!(let ReadFilterValue::Literal(_) = &read.filters[1].value);
    }

    // -----------------------------------------------------------------------
    // Paged read action tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_action_with_paged() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read all {
                paged;
            }
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        assert!(let Some(paged) = &read.paged);
        check!(paged.default_per_page.is_none());
        check!(paged.max_per_page.is_none());
    }

    #[test]
    fn read_action_with_paged_config() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read search {
                argument { status: Option<String> };
                filter { status == arg(status) };
                paged {
                    default_per_page 50;
                    max_per_page 200;
                };
            }
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.arguments.len() == 1);
        check!(read.filters.len() == 1);
        assert!(let Some(paged) = &read.paged);
        check!(paged.default_per_page == Some(50));
        check!(paged.max_per_page == Some(200));
    }

    #[test]
    fn read_action_with_paged_partial_config() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read all {
                paged {
                    max_per_page 500;
                };
            }
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        assert!(let Some(paged) = &read.paged);
        check!(paged.default_per_page.is_none());
        check!(paged.max_per_page == Some(500));
    }

    #[test]
    fn read_action_without_paged() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read all;
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.paged.is_none());
    }

    #[test]
    fn read_action_paged_invalid_key_produces_error() {
        let result = syn::parse2::<ResourceActionInput>(quote! {
            read all {
                paged {
                    bogus 42;
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("Unexpected paged config key"));
        check!(msg.contains("bogus"));
    }

    // -----------------------------------------------------------------------
    // Relation parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn resource_with_belongs_to_relation() {
        let input = parse_resource(quote! {
            name = Comment;

            attributes {
                id String;
                author_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                };
            }
        });

        check!(input.relations.len() == 1);
        let rel = &input.relations[0];
        check!(rel.name == "author");
        check!(rel.kind == RelationKind::BelongsTo);
        check!(rel.source_attribute == "author_id");
        check!(rel.destination_attribute.is_none());
    }

    #[test]
    fn resource_with_has_many_relation() {
        let input = parse_resource(quote! {
            name = Post;

            attributes {
                id String;
            }

            relations {
                has_many comments {
                    ty Comment;
                    source_attribute post_id;
                };
            }
        });

        check!(input.relations.len() == 1);
        let rel = &input.relations[0];
        check!(rel.name == "comments");
        check!(rel.kind == RelationKind::HasMany);
        check!(rel.source_attribute == "post_id");
        check!(rel.destination_attribute.is_none());
    }

    #[test]
    fn resource_with_multiple_relations() {
        let input = parse_resource(quote! {
            name = Comment;

            attributes {
                id String;
                author_id String;
                post_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                };
                belongs_to post {
                    ty Post;
                    source_attribute post_id;
                };
            }
        });

        check!(input.relations.len() == 2);
        check!(input.relations[0].name == "author");
        check!(input.relations[0].kind == RelationKind::BelongsTo);
        check!(input.relations[1].name == "post");
        check!(input.relations[1].kind == RelationKind::BelongsTo);
    }

    #[test]
    fn relation_with_destination_attribute() {
        let input = parse_resource(quote! {
            name = Comment;

            attributes {
                id String;
                author_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                    destination_attribute user_id;
                };
            }
        });

        check!(input.relations.len() == 1);
        assert!(let Some(dest) = &input.relations[0].destination_attribute);
        check!(*dest == "user_id");
    }

    #[test]
    fn relation_missing_ty_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Comment;

            attributes {
                id String;
            }

            relations {
                belongs_to author {
                    source_attribute author_id;
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("missing required `ty`"));
    }

    #[test]
    fn relation_missing_source_attribute_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Comment;

            attributes {
                id String;
            }

            relations {
                belongs_to author {
                    ty User;
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("missing required `source_attribute`"));
    }

    #[test]
    fn relation_unknown_key_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Comment;

            attributes {
                id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                    bogus foo;
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("Unexpected relation key"));
        check!(msg.contains("bogus"));
    }

    #[test]
    fn unknown_relation_kind_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Comment;

            attributes {
                id String;
            }

            relations {
                many_to_many tags {
                    ty Tag;
                    source_attribute tag_id;
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("Unexpected relation kind"));
        check!(msg.contains("many_to_many"));
    }

    #[test]
    fn resource_with_no_relations_block() {
        let input = parse_resource(quote! {
            name = Simple;

            attributes {
                id String;
            }
        });

        check!(input.relations.is_empty());
    }

    #[test]
    fn resource_with_empty_relations_block() {
        let input = parse_resource(quote! {
            name = Simple;

            attributes {
                id String;
            }

            relations {}
        });

        check!(input.relations.is_empty());
    }

    // -----------------------------------------------------------------------
    // Load keyword tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_action_with_load() {
        let input = parse_resource(quote! {
            name = Comment;

            attributes {
                id String;
                author_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                };
            }

            actions {
                read all_with_author {
                    load [author];
                };
            }
        });

        check!(input.actions.len() == 1);
        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.load.len() == 1);
        check!(read.load[0] == "author");
    }

    #[test]
    fn read_action_with_multiple_loads() {
        let input = parse_resource(quote! {
            name = Post;

            attributes {
                id String;
                author_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                };
                has_many comments {
                    ty Comment;
                    source_attribute post_id;
                };
            }

            actions {
                read full {
                    load [author, comments];
                };
            }
        });

        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.load.len() == 2);
        check!(read.load[0] == "author");
        check!(read.load[1] == "comments");
    }

    #[test]
    fn read_action_without_load_has_empty_load_vec() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read all;
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.load.is_empty());
    }

    #[test]
    fn read_action_load_with_filters_and_paged() {
        let input = parse_resource(quote! {
            name = Comment;

            attributes {
                id String;
                author_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                };
            }

            actions {
                read search_with_author {
                    argument { status: Option<String> };
                    filter { status == arg(status) };
                    load [author];
                    paged;
                };
            }
        });

        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.arguments.len() == 1);
        check!(read.filters.len() == 1);
        check!(read.load.len() == 1);
        check!(read.load[0] == "author");
        check!(read.paged.is_some());
    }

    #[test]
    fn load_referencing_undeclared_relation_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Comment;

            attributes {
                id String;
            }

            actions {
                read with_author {
                    load [author];
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("undeclared relation"));
        check!(msg.contains("author"));
    }

    #[test]
    fn load_referencing_nonexistent_relation_with_relations_block_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Comment;

            attributes {
                id String;
                author_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                };
            }

            actions {
                read with_tags {
                    load [tags];
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("undeclared relation"));
        check!(msg.contains("tags"));
    }

    #[test]
    fn relations_block_works_with_actions_and_extensions() {
        let input = parse_resource(quote! {
            name = Comment;

            attributes {
                id String;
                author_id String;
            }

            relations {
                belongs_to author {
                    ty User;
                    source_attribute author_id;
                };
            }

            actions {
                read all;
                read all_with_author {
                    load [author];
                };
            }

            extensions {
                cinderblock_json_api {
                    list = true;
                };
            }
        });

        check!(input.relations.len() == 1);
        check!(input.actions.len() == 2);
        check!(input.extensions.len() == 1);

        // First read action has no loads
        assert!(let ResourceActionInputKind::Read(read_all) = &input.actions[0].kind);
        check!(read_all.load.is_empty());

        // Second read action loads author
        assert!(let ResourceActionInputKind::Read(read_with_author) = &input.actions[1].kind);
        check!(read_with_author.load.len() == 1);
        check!(read_with_author.load[0] == "author");
    }

    // -----------------------------------------------------------------------
    // Order keyword tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_action_with_single_order_desc() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
                created_at String;
            }

            actions {
                read recent {
                    order { created_at desc; };
                };
            }
        });

        check!(input.actions.len() == 1);
        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.orders.len() == 1);
        check!(read.orders[0].field == "created_at");
        check!(let OrderDirection::Desc = read.orders[0].direction);
    }

    #[test]
    fn read_action_with_single_order_asc() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
                title String;
            }

            actions {
                read alphabetical {
                    order { title asc; };
                };
            }
        });

        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.orders.len() == 1);
        check!(read.orders[0].field == "title");
        check!(let OrderDirection::Asc = read.orders[0].direction);
    }

    #[test]
    fn read_action_with_order_default_direction() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
                title String;
            }

            actions {
                read alphabetical {
                    order { title; };
                };
            }
        });

        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.orders.len() == 1);
        check!(read.orders[0].field == "title");
        check!(let OrderDirection::Asc = read.orders[0].direction);
    }

    #[test]
    fn read_action_with_compound_order() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
                priority String;
                title String;
            }

            actions {
                read sorted {
                    order { priority desc; title asc; };
                };
            }
        });

        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.orders.len() == 2);
        check!(read.orders[0].field == "priority");
        check!(let OrderDirection::Desc = read.orders[0].direction);
        check!(read.orders[1].field == "title");
        check!(let OrderDirection::Asc = read.orders[1].direction);
    }

    #[test]
    fn read_action_with_order_and_filter_and_paged() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                id String;
                title String;
                done bool;
            }

            actions {
                read open_sorted {
                    filter { done == false };
                    order { title asc; };
                    paged;
                };
            }
        });

        assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
        check!(read.filters.len() == 1);
        check!(read.orders.len() == 1);
        check!(read.paged.is_some());
    }

    #[test]
    fn read_action_without_order_has_empty_orders() {
        assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
            read all;
        }));

        assert!(let ResourceActionInputKind::Read(read) = &action.kind);
        check!(read.orders.is_empty());
    }

    #[test]
    fn order_with_invalid_direction_produces_error() {
        let result = syn::parse2::<ResourceActionInput>(quote! {
            read broken {
                order { title backwards; };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("Unexpected order direction"));
        check!(msg.contains("backwards"));
    }

    #[test]
    fn order_referencing_undeclared_attribute_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Ticket;

            attributes {
                id String;
            }

            actions {
                read sorted {
                    order { nonexistent desc; };
                };
            }
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("undeclared attribute"));
        check!(msg.contains("nonexistent"));
    }

    // -----------------------------------------------------------------------
    // uuid_primary_key shortcut tests
    // -----------------------------------------------------------------------

    #[test]
    fn uuid_primary_key_shortcut() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                ticket_id uuid_primary_key;
            }
        });

        check!(input.attributes.len() == 1);
        let attr = &input.attributes[0];
        check!(attr.name == "ticket_id");
        check!(attr.primary_key.value());
        check!(!attr.writable.value());
        check!(attr.default.is_some());

        let ty = &attr.ty;
        let ty_str = quote::quote! { #ty }.to_string();
        check!(ty_str.contains("Uuid"));
    }

    #[test]
    fn uuid_primary_key_with_other_attributes() {
        let input = parse_resource(quote! {
            name = Order;

            attributes {
                order_id uuid_primary_key;
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
    fn uuid_primary_key_with_override_block() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                ticket_id uuid_primary_key {
                    generated true;
                }
            }
        });

        let attr = &input.attributes[0];
        check!(attr.primary_key.value());
        check!(!attr.writable.value());
        check!(attr.generated.value());
        check!(attr.default.is_some());
    }

    #[test]
    fn uuid_primary_key_override_writable() {
        let input = parse_resource(quote! {
            name = Ticket;

            attributes {
                ticket_id uuid_primary_key {
                    writable true;
                }
            }
        });

        let attr = &input.attributes[0];
        check!(attr.writable.value());
        check!(attr.primary_key.value());
        check!(attr.default.is_some());
    }

    #[test]
    fn parse_before_create_hook() {
        let input = parse_resource(quote! {
            name = Post;

            attributes {
                id String;
            }

            before_create |post| {
                post.id = String::from("generated");
            };
        });

        check!(input.before_create.is_some());
        check!(input.before_update.is_none());
    }

    #[test]
    fn parse_before_update_hook() {
        let input = parse_resource(quote! {
            name = Post;

            attributes {
                id String;
            }

            before_update |post| {
                post.id = String::from("updated");
            };
        });

        check!(input.before_create.is_none());
        check!(input.before_update.is_some());
    }

    #[test]
    fn parse_both_hooks() {
        let input = parse_resource(quote! {
            name = Post;

            attributes {
                id String;
            }

            before_create |post| {
                post.id = String::from("created");
            };

            before_update |post| {
                post.id = String::from("updated");
            };
        });

        check!(input.before_create.is_some());
        check!(input.before_update.is_some());
    }

    #[test]
    fn hooks_work_alongside_actions() {
        let input = parse_resource(quote! {
            name = Post;

            attributes {
                id String;
            }

            before_create |post| {
                post.id = String::from("created");
            };

            actions {
                create publish;
                update edit;
            }

            before_update |post| {
                post.id = String::from("updated");
            };
        });

        check!(input.before_create.is_some());
        check!(input.before_update.is_some());
        check!(input.actions.len() == 2);
    }

    #[test]
    fn duplicate_before_create_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Post;

            attributes {
                id String;
            }

            before_create |post| {
                post.id = String::from("first");
            };

            before_create |post| {
                post.id = String::from("second");
            };
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("before_create"));
        check!(msg.contains("once"));
    }

    #[test]
    fn duplicate_before_update_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Post;

            attributes {
                id String;
            }

            before_update |post| {
                post.id = String::from("first");
            };

            before_update |post| {
                post.id = String::from("second");
            };
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("before_update"));
        check!(msg.contains("once"));
    }

    #[test]
    fn no_hooks_declared_defaults_to_none() {
        let input = parse_resource(quote! {
            name = Simple;

            attributes {
                id u64;
            }
        });

        check!(input.before_create.is_none());
        check!(input.before_update.is_none());
    }
}
