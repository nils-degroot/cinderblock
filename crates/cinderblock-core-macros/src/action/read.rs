use cinderblock_extension_api::{
    ActionRead, OrderDirection, ReadFilterValue, RelationDecl, RelationKind, util::is_optional,
};
use syn::Ident;

use crate::action::ActionGenerateContext;

pub(crate) fn generate_read(
    ctx: &ActionGenerateContext,
    read_action: &ActionRead,
) -> proc_macro2::TokenStream {
    let is_get = read_action.get;
    let is_paged = read_action.paged.is_some();
    let has_loads = !read_action.load.is_empty();

    let action_name = ctx.action.action_name.clone();

    let resource_name = ctx.resource_name.clone();

    // # Get-action fast path
    //
    // Get-actions use `PrimaryKey` as Arguments and the resource
    // itself as Response. No Arguments struct, no filter codegen,
    // no in-memory data layer traits — the blanket
    // `PerformReadOne` impl on InMemoryDataLayer handles it.
    if is_get && !has_loads {
        return quote::quote! {
            struct #action_name;

            impl cinderblock_core::ReadAction for #action_name {
                type Output = #resource_name;
                type Arguments = <#resource_name as cinderblock_core::Resource>::PrimaryKey;
                type Response = #resource_name;
            }
        };
    }

    // # Get-action with relation loading
    //
    // When `get` is combined with `load [...]`, we generate a
    // response wrapper and a `PerformReadOne` impl that fetches the
    // base resource by primary key, then loads related resources
    // and assembles the wrapper.
    if is_get && has_loads {
        let loaded_relations: Vec<&RelationDecl> = read_action
            .load
            .iter()
            .map(|name| {
                ctx.input
                    .relations
                    .iter()
                    .find(|r| r.name == *name)
                    .expect("load reference validated during parsing")
            })
            .collect();

        let wrapper_name = Ident::new(
            &format!("{action_name}Response"),
            ctx.action.action_name.span(),
        );

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

        let response_wrapper = quote::quote! {
            #[derive(::std::fmt::Debug, ::std::clone::Clone, cinderblock_core::serde::Serialize)]
            pub struct #wrapper_name {
                #[serde(flatten)]
                pub base: #resource_name,
                #(#relation_fields),*
            }
        };

        let relation_loads = loaded_relations.iter().map(|rel| {
            let rel_ty = &rel.ty;
            let source_attr = &rel.source_attribute;
            let map_name = Ident::new(&format!("{}_map", rel.name), rel.name.span());

            match rel.kind {
                RelationKind::BelongsTo => {
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
                                        .get(&base.#source_attr.to_string())
                                        .cloned()
                                        .ok_or_else(|| {
                                            cinderblock_core::ReadError::DataLayer(
                                                format!(
                                                    "belongs_to relation `{}` of type `{}`: no record found for FK value `{}`",
                                                    #rel_name_str,
                                                    ::std::any::type_name::<#rel_ty>(),
                                                    base.#source_attr,
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
                                            .get(&base.primary_key().to_string())
                                            .cloned()
                                            .unwrap_or_default()
                                    }
                                }
                            }
                        }
                    });

        let data_layer_block = if ctx.input.data_layer.is_some() {
            quote::quote! {}
        } else {
            quote::quote! {
                impl cinderblock_core::PerformReadOne<#action_name> for cinderblock_core::data_layer::in_memory::InMemoryDataLayer {
                    async fn read_one(&self, args: &<#action_name as cinderblock_core::ReadAction>::Arguments) -> Result<<#action_name as cinderblock_core::ReadAction>::Response, cinderblock_core::ReadError> {
                        let dl = self;

                        let base = <Self as cinderblock_core::data_layer::DataLayer<#resource_name>>::read(dl, args).await?;

                        #(#relation_loads)*

                        Ok(#wrapper_name {
                            #(#relation_field_inits,)*
                            base,
                        })
                    }
                }
            }
        };

        return quote::quote! {
            #response_wrapper

            pub struct #action_name;

            impl cinderblock_core::ReadAction for #action_name {
                type Output = #resource_name;
                type Arguments = <#resource_name as cinderblock_core::Resource>::PrimaryKey;
                type Response = #wrapper_name;
            }

            #data_layer_block
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
        let args_name = Ident::new(
            &format!("{action_name}Arguments"),
            ctx.action.raw_name.span(),
        );
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
        let args_name = Ident::new(
            &format!("{action_name}Arguments"),
            ctx.action.raw_name.span(),
        );

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
            ctx.input
                .relations
                .iter()
                .find(|r| r.name == *name)
                .expect("load reference validated during parsing")
        })
        .collect();

    let response_wrapper = if has_loads {
        let wrapper_name = Ident::new(
            &format!("{action_name}Response"),
            ctx.action.raw_name.span(),
        );

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
                pub base: #resource_name,
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
        let wrapper_name = Ident::new(
            &format!("{action_name}Response"),
            ctx.action.raw_name.span(),
        );
        quote::quote! { Vec<#wrapper_name> }
    } else if is_paged {
        quote::quote! { cinderblock_core::PaginatedResult<#resource_name> }
    } else {
        quote::quote! { Vec<#resource_name> }
    };

    // # Filter codegen helper
    //
    // Builds the chain of boolean clauses for the row predicate.
    // Used by both the plain and relation-loading code paths.
    let build_filters = |read_action: &cinderblock_extension_api::ActionRead| {
        read_action
            .filters
            .iter()
            .map(|filter| {
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

                        if is_optional(&arg_decl.ty) {
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
            })
            .collect::<Vec<_>>()
    };

    // # Order comparator codegen helper
    //
    // Builds a sort_by closure body from the read action's `order`
    // clauses. Each clause becomes a `.cmp()` call on the field,
    // chained via `.then_with()` for compound ordering.
    //
    // Returns `None` when there are no order clauses, so callers
    // can skip emitting sort code entirely.
    let build_order_sort =
        |read_action: &cinderblock_extension_api::ActionRead| -> Option<proc_macro2::TokenStream> {
            if read_action.orders.is_empty() {
                return None;
            }

            let mut clauses = read_action.orders.iter();
            let first = clauses.next().unwrap();
            let first_field = &first.field;
            let first_cmp = match first.direction {
                OrderDirection::Asc => quote::quote! { a.#first_field.cmp(&b.#first_field) },
                OrderDirection::Desc => quote::quote! { b.#first_field.cmp(&a.#first_field) },
            };

            let rest: Vec<_> = clauses
                .map(|clause| {
                    let field = &clause.field;
                    match clause.direction {
                        OrderDirection::Asc => {
                            quote::quote! { .then_with(|| a.#field.cmp(&b.#field)) }
                        }
                        OrderDirection::Desc => {
                            quote::quote! { .then_with(|| b.#field.cmp(&a.#field)) }
                        }
                    }
                })
                .collect();

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
    let data_layer_block = if ctx.input.data_layer.is_some() {
        quote::quote! {}
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
        let order_sort_block = order_sort
            .as_ref()
            .map(|cmp_body| {
                quote::quote! { base_rows.sort_by(|a, b| #cmp_body); }
            })
            .unwrap_or_default();

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
            let map_name = Ident::new(&format!("{}_map", rel.name), rel.name.span());

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
        let wrapper_name = Ident::new(
            &format!("{action_name}Response"),
            ctx.action.raw_name.span(),
        );

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
                    let mut base_rows: Vec<#resource_name> = dl.load_all::<#resource_name>().await
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
        let order_sort_block = order_sort
            .as_ref()
            .map(|cmp_body| {
                quote::quote! { filtered.sort_by(|a, b| #cmp_body); }
            })
            .unwrap_or_default();

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
            type Output = #resource_name;

            type Arguments = #arguments_type;

            type Response = #response_type;
        }

        #data_layer_block
    }
}
