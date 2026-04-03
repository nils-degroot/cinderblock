use core::iter::Iterator;
use std::collections::{HashMap, HashSet};

use cinderblock_extension_api::{
    Accept, OrderDirection, ReadFilterValue, RelationDecl, RelationKind, ResourceActionInputKind,
    ResourceAttributeInput, ResourceMacroInput, UpdateChange,
};
use syn::{Ident, Type, spanned::Spanned};

/// Checks whether a `syn::Type` is `Option<T>`.
///
/// We inspect the outermost path segment for the identifier `Option`. This
/// handles both `Option<T>` and `std::option::Option<T>` (by checking the
/// last segment).
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

#[proc_macro]
pub fn resource(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    // Capture the raw input tokens before parsing so we can forward them
    // verbatim to extension macros without a reconstruct roundtrip.
    let raw_tokens: proc_macro2::TokenStream = item.clone().into();

    let input = syn::parse_macro_input!(item as ResourceMacroInput);

    let ident = input.name.last().expect("Missing name segment");

    let fields = input
        .attributes
        .iter()
        .map(ResourceAttributeInput::to_field_definition);

    let primary_key_type = {
        let fields = input
            .attributes
            .iter()
            .filter(|attr| attr.primary_key.value())
            .collect::<Vec<_>>();

        match fields.len() {
            0 => todo!("Support no primary keys"),
            1 => {
                let ty = fields[0].ty.clone();
                quote::quote! { #ty }
            }
            _ => todo!("Support multiple primary keys"),
        }
    };

    let primary_key_generated = {
        let fields = input
            .attributes
            .iter()
            .filter(|attr| attr.primary_key.value())
            .collect::<Vec<_>>();

        match fields.len() {
            0 => todo!("Support no primary keys"),
            1 => fields[0].generated.value(),
            _ => todo!("Support multiple primary keys"),
        }
    };

    let primary_key_value = {
        let fields = input
            .attributes
            .iter()
            .filter(|attr| attr.primary_key.value())
            .collect::<Vec<_>>();

        match fields.len() {
            0 => todo!("Support no primary keys"),
            1 => {
                let ty = fields[0].name.clone();
                quote::quote! { &self.#ty }
            }
            _ => todo!("Support multiple primary keys"),
        }
    };

    let data_layer_specified = input.data_layer.is_some();

    let relations = &input.relations;

    let actions = input.actions.iter().map(|action| match &action.kind {
        ResourceActionInputKind::Read(read_action) => {
            let action_name = convert_case::ccase!(pascal, action.name.to_string());
            let action_name = Ident::new(&action_name, action.name.span());

            let is_get = read_action.get;
            let is_paged = read_action.paged.is_some();
            let has_loads = !read_action.load.is_empty();

            // # Get-action fast path
            //
            // Get-actions use `PrimaryKey` as Arguments and the resource
            // itself as Response. No Arguments struct, no filter codegen,
            // no in-memory data layer traits — the blanket
            // `PerformReadOne` impl on InMemoryDataLayer handles it.
            if is_get {
                return quote::quote! {
                    struct #action_name;

                    impl cinderblock_core::ReadAction for #action_name {
                        type Output = #ident;
                        type Arguments = <#ident as cinderblock_core::Resource>::PrimaryKey;
                        type Response = #ident;
                    }
                };
            }

            // # Arguments struct generation
            //
            // When the read action declares arguments, we generate a dedicated
            // `{ActionName}Arguments` struct with `Deserialize` so it can be
            // populated from query parameters. When no arguments are declared,
            // we use `()` as the arguments type — unless the action is paged,
            // in which case we always need an Arguments struct to hold
            // `page` and `per_page` fields.
            let has_user_arguments = !read_action.arguments.is_empty();
            let needs_arguments_struct = has_user_arguments || is_paged;

            let (arguments_type, arguments_struct) = if needs_arguments_struct {
                let args_name = Ident::new(&format!("{action_name}Arguments"), action.name.span());
                let user_arg_fields = read_action.arguments.iter().map(|arg| {
                    let name = &arg.name;
                    let ty = &arg.ty;
                    quote::quote! { pub #name: #ty }
                });

                // For paged actions, append `page` and `per_page` fields.
                let paged_fields = if is_paged {
                    quote::quote! {
                        pub page: Option<u32>,
                        pub per_page: Option<u32>,
                    }
                } else {
                    quote::quote! {}
                };

                (
                    quote::quote! { #args_name },
                    quote::quote! {
                        #[derive(::std::fmt::Debug, cinderblock_core::serde::Deserialize)]
                        pub struct #args_name {
                            #(#user_arg_fields,)*
                            #paged_fields
                        }
                    },
                )
            } else {
                (quote::quote! { () }, quote::quote! {})
            };

            // # Paged trait impl
            //
            // When the action is paged, generate a `Paged` impl on the
            // Arguments struct that resolves defaults and clamping from
            // the DSL config.
            let paged_impl = if let Some(paged_config) = &read_action.paged {
                let args_name = Ident::new(&format!("{action_name}Arguments"), action.name.span());

                let default_per_page = match paged_config.default_per_page {
                    Some(n) => quote::quote! { #n },
                    None => quote::quote! { cinderblock_core::DEFAULT_PER_PAGE },
                };

                // Clamp per_page to max if configured, otherwise just use
                // the resolved value directly.
                let per_page_body = if let Some(max) = paged_config.max_per_page {
                    quote::quote! {
                        self.per_page.unwrap_or(#default_per_page).min(#max)
                    }
                } else {
                    quote::quote! {
                        self.per_page.unwrap_or(#default_per_page)
                    }
                };

                quote::quote! {
                    impl cinderblock_core::Paged for #args_name {
                        fn page(&self) -> u32 {
                            self.page.unwrap_or(1)
                        }

                        fn per_page(&self) -> u32 {
                            #per_page_body
                        }
                    }
                }
            } else {
                quote::quote! {}
            };

            // # Response wrapper struct generation
            //
            // When a read action declares `load [...]`, we generate a
            // response wrapper struct that flattens the base resource and
            // adds a field for each loaded relation. The wrapper is used as
            // the `Response` element type instead of the raw resource.
            //
            // For `belongs_to`, the field type is the related resource.
            // For `has_many`, the field type is `Vec<RelatedResource>`.
            let loaded_relations: Vec<&RelationDecl> = read_action
                .load
                .iter()
                .map(|name| {
                    relations
                        .iter()
                        .find(|r| r.name == *name)
                        .expect("load reference validated during parsing")
                })
                .collect();

            let response_wrapper = if has_loads {
                let wrapper_name =
                    Ident::new(&format!("{action_name}Response"), action.name.span());

                let relation_fields = loaded_relations.iter().map(|rel| {
                    let rel_name = &rel.name;
                    let rel_ty = &rel.ty;
                    match rel.kind {
                        RelationKind::BelongsTo => quote::quote! {
                            pub #rel_name: #rel_ty
                        },
                        RelationKind::HasMany => quote::quote! {
                            pub #rel_name: Vec<#rel_ty>
                        },
                    }
                });

                quote::quote! {
                    #[derive(::std::fmt::Debug, ::std::clone::Clone, cinderblock_core::serde::Serialize)]
                    pub struct #wrapper_name {
                        #[serde(flatten)]
                        pub base: #ident,
                        #(#relation_fields),*
                    }
                }
            } else {
                quote::quote! {}
            };

            // # Response type
            //
            // Non-paged actions without `load` return `Vec<Output>`.
            // Non-paged actions with `load` return `Vec<WrapperStruct>`.
            // Paged actions without `load` return `PaginatedResult<Output>`.
            //
            // TODO: support paged + load combination
            let response_type = if has_loads {
                let wrapper_name =
                    Ident::new(&format!("{action_name}Response"), action.name.span());
                quote::quote! { Vec<#wrapper_name> }
            } else if is_paged {
                quote::quote! { cinderblock_core::PaginatedResult<#ident> }
            } else {
                quote::quote! { Vec<#ident> }
            };

            // # Filter codegen helper
            //
            // Builds the chain of boolean clauses for the row predicate.
            // Used by both the plain and relation-loading code paths.
            let build_filters = |read_action: &cinderblock_extension_api::ActionRead| {
                read_action.filters.iter().map(|filter| {
                    let field = &filter.field;
                    let op = match filter.op {
                        cinderblock_extension_api::ReadFilterOperation::Eq => quote::quote! { == },
                    };
                    match &filter.value {
                        ReadFilterValue::Literal(expr) => {
                            quote::quote! {
                                row.#field #op #expr &&
                            }
                        }
                        ReadFilterValue::Arg(arg_name) => {
                            let arg_decl = read_action
                                .arguments
                                .iter()
                                .find(|a| a.name == *arg_name)
                                .expect("arg reference validated during parsing");
                            if is_option_type(&arg_decl.ty) {
                                quote::quote! {
                                    args.#arg_name.as_ref().map_or(true, |v| row.#field #op *v) &&
                                }
                            } else {
                                quote::quote! {
                                    row.#field #op args.#arg_name &&
                                }
                            }
                        }
                    }
                }).collect::<Vec<_>>()
            };

            // # Order comparator codegen helper
            //
            // Builds a sort_by closure body from the read action's `order`
            // clauses. Each clause becomes a `.cmp()` call on the field,
            // chained via `.then_with()` for compound ordering.
            //
            // Returns `None` when there are no order clauses, so callers
            // can skip emitting sort code entirely.
            let build_order_sort = |read_action: &cinderblock_extension_api::ActionRead| -> Option<proc_macro2::TokenStream> {
                if read_action.orders.is_empty() {
                    return None;
                }

                let mut clauses = read_action.orders.iter();
                let first = clauses.next().unwrap();
                let first_field = &first.field;
                let first_cmp = match first.direction {
                    OrderDirection::Asc  => quote::quote! { a.#first_field.cmp(&b.#first_field) },
                    OrderDirection::Desc => quote::quote! { b.#first_field.cmp(&a.#first_field) },
                };

                let rest: Vec<_> = clauses.map(|clause| {
                    let field = &clause.field;
                    match clause.direction {
                        OrderDirection::Asc  => quote::quote! { .then_with(|| a.#field.cmp(&b.#field)) },
                        OrderDirection::Desc => quote::quote! { .then_with(|| b.#field.cmp(&a.#field)) },
                    }
                }).collect();

                Some(quote::quote! { #first_cmp #(#rest)* })
            };

            // # In-memory data layer codegen
            //
            // When no custom data layer is specified, generate the
            // InMemoryReadAction filter and either:
            //   - `InMemoryPerformRead` (for actions without `load`), or
            //   - A direct `PerformRead` impl (for actions with `load`)
            //     that does the base query then loads relations from the
            //     in-memory store.
            let data_layer_block = if data_layer_specified {
                quote::quote! { }
            } else if has_loads {
                // # Relation-loading PerformRead codegen
                //
                // For actions with `load`, we generate a direct `PerformRead`
                // impl on `InMemoryDataLayer` (skipping the blanket) that:
                //   1. Reads all base rows and applies filters
                //   2. Sorts results according to `order` clauses
                //   3. Loads each related resource type from the store
                //   4. Assembles the response wrapper for each base row
                let filters = build_filters(read_action);
                let order_sort = build_order_sort(read_action);
                let order_sort_block = order_sort.as_ref().map(|cmp_body| {
                    quote::quote! { base_rows.sort_by(|a, b| #cmp_body); }
                }).unwrap_or_default();

                // Generate the relation loading code. For each loaded relation:
                //
                // - `belongs_to`: collect FK values from base rows, load all
                //   destination resources, build a HashMap<PK, Resource>,
                //   then look up each base row's FK.
                //
                // - `has_many`: collect PK values from base rows, load all
                //   destination resources, group them by their FK field into
                //   a HashMap<FK, Vec<Resource>>.
                let relation_loads = loaded_relations.iter().map(|rel| {
                    let rel_ty = &rel.ty;
                    let source_attr = &rel.source_attribute;
                    let map_name = Ident::new(
                        &format!("{}_map", rel.name),
                        rel.name.span(),
                    );

                    match rel.kind {
                        RelationKind::BelongsTo => {
                            // For belongs_to: the FK is on the base resource.
                            // We load all destination resources and index them
                            // by their primary key.
                            quote::quote! {
                                let all_related: Vec<#rel_ty> = dl.load_all::<#rel_ty>().await;
                                let #map_name: ::std::collections::HashMap<String, #rel_ty> = all_related
                                    .into_iter()
                                    .map(|r| {
                                        use cinderblock_core::Resource;
                                        (r.primary_key().to_string(), r)
                                    })
                                    .collect();
                            }
                        }
                        RelationKind::HasMany => {
                            // For has_many: the FK is on the related resource,
                            // pointing back to the base resource's PK. We load
                            // all related resources and group them by the FK
                            // field value.
                            quote::quote! {
                                let all_related: Vec<#rel_ty> = dl.load_all::<#rel_ty>().await;
                                let mut #map_name: ::std::collections::HashMap<String, Vec<#rel_ty>> =
                                    ::std::collections::HashMap::new();
                                for r in all_related {
                                    let key = r.#source_attr.to_string();
                                    #map_name.entry(key).or_default().push(r);
                                }
                            }
                        }
                    }
                });

                // # Wrapper assembly for each base row
                //
                // For each filtered base row, look up the loaded relations
                // and construct the response wrapper struct.
                let wrapper_name =
                    Ident::new(&format!("{action_name}Response"), action.name.span());

                let relation_field_inits = loaded_relations.iter().map(|rel| {
                    let rel_name = &rel.name;
                    let source_attr = &rel.source_attribute;
                    let map_name = Ident::new(
                        &format!("{}_map", rel.name),
                        rel.name.span(),
                    );

                    match rel.kind {
                        RelationKind::BelongsTo => {
                            let rel_ty = &rel.ty;
                            let rel_name_str = rel.name.to_string();
                            quote::quote! {
                                #rel_name: #map_name
                                    .get(&row.#source_attr.to_string())
                                    .cloned()
                                    .ok_or_else(|| {
                                        cinderblock_core::ListError::DataLayer(
                                            format!(
                                                "belongs_to relation `{}` of type `{}`: no record found for FK value `{}`",
                                                #rel_name_str,
                                                ::std::any::type_name::<#rel_ty>(),
                                                row.#source_attr,
                                            ).into(),
                                        )
                                    })?
                            }
                        }
                        RelationKind::HasMany => {
                            quote::quote! {
                                #rel_name: {
                                    use cinderblock_core::Resource;
                                    #map_name
                                        .get(&row.primary_key().to_string())
                                        .cloned()
                                        .unwrap_or_default()
                                }
                            }
                        }
                    }
                });

                quote::quote! {
                    impl cinderblock_core::PerformRead<#action_name> for cinderblock_core::data_layer::in_memory::InMemoryDataLayer {
                        async fn read(&self, args: &<#action_name as cinderblock_core::ReadAction>::Arguments) -> Result<<#action_name as cinderblock_core::ReadAction>::Response, cinderblock_core::ListError> {
                            let dl = self;

                            // Step 1: Load and filter base rows
                            let mut base_rows: Vec<#ident> = dl.load_all::<#ident>().await
                                .into_iter()
                                .filter(|row| { #(#filters)* true })
                                .collect();

                            // Step 1b: Sort base rows if order clauses declared
                            #order_sort_block

                            // Step 2: Load related resources
                            #(#relation_loads)*

                            // Step 3: Assemble response wrappers
                            let results: Result<Vec<#wrapper_name>, cinderblock_core::ListError> = base_rows
                                .into_iter()
                                .map(|row| -> Result<#wrapper_name, cinderblock_core::ListError> {
                                    Ok(#wrapper_name {
                                        #(#relation_field_inits,)*
                                        base: row,
                                    })
                                })
                                .collect();

                            results
                        }
                    }
                }
            } else if is_paged {
                let filters = build_filters(read_action);
                let order_sort = build_order_sort(read_action);
                let order_sort_block = order_sort.as_ref().map(|cmp_body| {
                    quote::quote! { filtered.sort_by(|a, b| #cmp_body); }
                }).unwrap_or_default();

                quote::quote! {
                    impl cinderblock_core::data_layer::in_memory::InMemoryPagedReadAction for #action_name {
                        fn filter(row: &Self::Output, args: &Self::Arguments) -> bool {
                            #(#filters)* true
                        }
                    }

                    impl cinderblock_core::data_layer::in_memory::InMemoryPerformRead for #action_name {
                        fn execute(
                            all: impl Iterator<Item = Self::Output>,
                            args: &Self::Arguments,
                        ) -> Self::Response {
                            use cinderblock_core::Paged;

                            let mut filtered: Vec<Self::Output> = all
                                .filter(|row| <Self as cinderblock_core::data_layer::in_memory::InMemoryPagedReadAction>::filter(row, args))
                                .collect();

                            #order_sort_block

                            let total = filtered.len() as u64;
                            let page = args.page();
                            let per_page = args.per_page();
                            let total_pages = if total == 0 { 1 } else { ((total as u32).saturating_add(per_page - 1)) / per_page };

                            let skip = ((page.saturating_sub(1)) as usize) * (per_page as usize);
                            let data: Vec<Self::Output> = filtered
                                .into_iter()
                                .skip(skip)
                                .take(per_page as usize)
                                .collect();

                            cinderblock_core::PaginatedResult {
                                data,
                                meta: cinderblock_core::PaginationMeta {
                                    page,
                                    per_page,
                                    total,
                                    total_pages,
                                },
                            }
                        }
                    }
                }
            } else {
                // # Filter codegen for InMemoryReadAction
                //
                // Each filter becomes a boolean clause AND'd together. Literal
                // values are emitted directly; `arg(name)` references access
                // the corresponding field on the args struct.
                //
                // For `Option<T>` argument types, the filter clause is
                // conditional: when `None`, the clause is skipped (evaluates
                // to `true`). This lets optional arguments act as "filter if
                // provided" semantics.
                let filters = build_filters(read_action);
                let order_sort = build_order_sort(read_action);

                let execute_body = if let Some(cmp_body) = order_sort {
                    quote::quote! {
                        let mut results: Vec<Self::Output> = all
                            .filter(|row| <Self as cinderblock_core::data_layer::in_memory::InMemoryReadAction>::filter(row, args))
                            .collect();
                        results.sort_by(|a, b| #cmp_body);
                        results
                    }
                } else {
                    quote::quote! {
                        all.filter(|row| <Self as cinderblock_core::data_layer::in_memory::InMemoryReadAction>::filter(row, args))
                            .collect()
                    }
                };

                quote::quote! {
                    impl cinderblock_core::data_layer::in_memory::InMemoryReadAction for #action_name {
                        fn filter(row: &Self::Output, args: &Self::Arguments) -> bool {
                            #(#filters)* true
                        }
                    }

                    impl cinderblock_core::data_layer::in_memory::InMemoryPerformRead for #action_name {
                        fn execute(
                            all: impl Iterator<Item = Self::Output>,
                            args: &Self::Arguments,
                        ) -> Self::Response {
                            #execute_body
                        }
                    }
                }
            };

            quote::quote! {
                #arguments_struct

                #paged_impl

                #response_wrapper

                pub struct #action_name;

                impl cinderblock_core::ReadAction for #action_name {
                    type Output = #ident;

                    type Arguments = #arguments_type;

                    type Response = #response_type;
                }

                #data_layer_block
            }
        }
        ResourceActionInputKind::Create { accept } => {
            let action_name = convert_case::ccase!(pascal, action.name.to_string());
            let action_name = Ident::new(&action_name, action.name.span());
            let input_name = Ident::new(&format!("{action_name}Input"), action.name.span());

            let attributes = input.attributes.iter().filter(|attr| attr.writable.value());

            let (present, mut missing_names) = match accept {
                Accept::Default => (
                    attributes
                        .map(|attr| (attr.name.to_string(), attr))
                        .collect::<HashMap<_, _>>(),
                    HashMap::new(),
                ),
                Accept::Only(idents) => {
                    let idents = idents
                        .iter()
                        .map(|ident| ident.to_string())
                        .collect::<HashSet<_>>();

                    attributes.fold(
                        (HashMap::new(), HashMap::new()),
                        |(mut present, mut missing), attr| {
                            if idents.contains(&attr.name.to_string()) {
                                present.insert(attr.name.to_string(), attr);
                            } else {
                                missing.insert(attr.name.to_string(), attr);
                            }
                            (present, missing)
                        },
                    )
                }
            };

            input
                .attributes
                .iter()
                .filter(|attr| {
                    !attr.writable.value() || !present.contains_key(&attr.name.to_string())
                })
                .for_each(|attr| {
                    missing_names.insert(attr.name.to_string(), attr);
                });

            let attributes = present.values().map(|attr| attr.to_field_definition());

            let missing_names = missing_names.values().map(|attr| attr.to_default());

            let present_names = present.values().map(|attr| attr.name.clone());

            quote::quote! {
                #[derive(::std::fmt::Debug)]
                pub struct #action_name;

                #[derive(::std::fmt::Debug, cinderblock_core::serde::Deserialize)]
                pub struct #input_name {
                    #(pub #attributes),*
                }

                impl cinderblock_core::Create<#action_name> for #ident {
                    type Input = #input_name;

                    fn from_create_input(input: Self::Input) -> Self {
                        #ident {
                            // Iterate over attributes in #action_name.
                            #(#present_names: input.#present_names,)*

                            // All types attributes not present in #action_name should use default
                            #(#missing_names),*
                        }
                    }
                }
            }
        }
        ResourceActionInputKind::Update(update) => {
            let action_name = convert_case::ccase!(pascal, action.name.to_string());
            let action_name = Ident::new(&action_name, action.name.span());
            let input_name = Ident::new(&format!("{action_name}Input"), action.name.span());

            let attributes = input.attributes.iter().filter(|attr| attr.writable.value());

            let present = match &update.accept {
                Accept::Default => attributes.collect::<Vec<_>>(),
                Accept::Only(idents) => {
                    let idents = idents
                        .iter()
                        .map(|ident| ident.to_string())
                        .collect::<HashSet<_>>();

                    attributes
                        .filter(|attr| idents.contains(&attr.name.to_string()))
                        .collect()
                }
            };

            let field_definitions = present.iter().map(|attr| attr.to_field_definition());

            // # Field assignment generation
            //
            // Each accepted field from the input struct gets assigned onto `self`
            // in the generated `apply_update_input` method.
            let field_assignments = present.iter().map(|attr| {
                let name = &attr.name;
                quote::quote! { self.#name = input.#name; }
            });

            // # Change closure generation
            //
            // Each `change_ref` closure is emitted as a typed closure bound to a
            // variable, then called with `self`. We inject `&mut Self` as the
            // parameter type so field access resolves without the user needing
            // to annotate the type in the DSL.
            let change_ref_calls =
                update
                    .changes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, change)| match change {
                        UpdateChange::ChangeRef(closure) => {
                            let param = closure
                                .inputs
                                .first()
                                .expect("change_ref closure must have exactly one parameter");
                            let body = &closure.body;
                            let binding = Ident::new(&format!("change_ref_{i}"), param.span());
                            Some(quote::quote! {
                                let #binding = |#param: &mut Self| #body;
                                #binding(self);
                            })
                        }
                        // TODO: support `change` (by-value) variant
                        UpdateChange::Change(_) => None,
                    });

            quote::quote! {
                #[derive(::std::fmt::Debug)]
                pub struct #action_name;

                #[derive(::std::fmt::Debug, cinderblock_core::serde::Deserialize)]
                pub struct #input_name {
                    #(pub #field_definitions),*
                }

                impl cinderblock_core::Update<#action_name> for #ident {
                    type Input = #input_name;

                    fn apply_update_input(&mut self, input: Self::Input) {
                        #(#field_assignments)*
                        #(#change_ref_calls)*
                    }
                }
            }
        }
        ResourceActionInputKind::Destroy => {
            // # Destroy action codegen
            //
            // Destroy actions only need a marker struct and a `Destroy<A>` impl.
            // No input struct is generated — the primary key comes from the
            // URL path at the HTTP layer.
            let action_name = convert_case::ccase!(pascal, action.name.to_string());
            let action_name = Ident::new(&action_name, action.name.span());

            quote::quote! {
                #[derive(::std::fmt::Debug)]
                pub struct #action_name;

                impl cinderblock_core::Destroy<#action_name> for #ident {}
            }
        }
    });

    let name_segments: Vec<String> = input
        .name
        .iter()
        .map(|segment| segment.to_string())
        .collect();
    let resource_name_literal = name_segments.join(".");

    // # Data layer selection
    //
    // If the user specified `data_layer = some::Path;` in the DSL, use that
    // path. Otherwise default to the built-in in-memory data layer.
    let data_layer_path = input.data_layer.map_or_else(
        || quote::quote! { cinderblock_core::data_layer::in_memory::InMemoryDataLayer },
        |path| quote::quote! { #path },
    );

    // # Extension forwarding
    //
    // For each declared extension, we forward the raw DSL tokens (captured
    // before parsing) inside a braced group, followed by a `config = { ... }`
    // block containing the extension-specific configuration. This avoids a
    // parse-then-reconstruct roundtrip — the extension macro receives the
    // exact tokens the user wrote.
    let extension_calls = input.extensions.iter().map(|ext| {
        let path = &ext.path;
        let config_tokens = &ext.config_tokens;

        quote::quote! {
            #path::__resource_extension! {
                { #raw_tokens }

                config = {
                    #config_tokens
                }
            }
        }
    });

    quote::quote! {
        #[derive(::std::fmt::Debug, ::std::clone::Clone, cinderblock_core::serde::Serialize, cinderblock_core::serde::Deserialize)]
        pub struct #ident {
            #(#fields),*
        }

        impl cinderblock_core::Resource for #ident {
            type PrimaryKey = #primary_key_type;

            type DataLayer = #data_layer_path;

            const NAME: &'static [&'static str] = &[#(#name_segments),*];

            const RESOURCE_NAME: &'static str = #resource_name_literal;

            const PRIMARY_KEY_GENERATED: bool = #primary_key_generated;

            fn primary_key(&self) -> &Self::PrimaryKey {
                #primary_key_value
            }
        }

        #(#actions)*

        #(#extension_calls)*
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert2::{assert, check};
    use cinderblock_extension_api::ResourceActionInput;
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
        // writable should still be the default (true) since it wasn't set.
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
}
