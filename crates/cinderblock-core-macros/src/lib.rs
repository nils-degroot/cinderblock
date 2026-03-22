use core::iter::Iterator;
use std::collections::{HashMap, HashSet};

use cinderblock_extension_api::{
    Accept, ResourceActionInputKind, ResourceAttributeInput, ResourceMacroInput, UpdateChange,
};
use syn::{spanned::Spanned, Ident};

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

    let actions = input.actions.iter().map(|action| match &action.kind {
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
                struct #action_name;

                #[derive(::std::fmt::Debug, cinderblock_core::serde::Deserialize)]
                struct #input_name {
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
                struct #action_name;

                #[derive(::std::fmt::Debug, cinderblock_core::serde::Deserialize)]
                struct #input_name {
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
                struct #action_name;

                impl cinderblock_core::Destroy<#action_name> for #ident {}
            }
        }
    });

    let name_segments = input.name.iter().map(|segment| segment.to_string());

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
        struct #ident {
            #(#fields),*
        }

        impl cinderblock_core::Resource for #ident {
            type PrimaryKey = #primary_key_type;

            type DataLayer = #data_layer_path;

            const NAME: &'static [&'static str] = &[#(#name_segments),*];

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
    use cinderblock_extension_api::ResourceActionInput;
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
