use core::iter::Iterator;
use std::collections::{HashMap, HashSet};

use ash_extension_api::{
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

                #[derive(::std::fmt::Debug, ash_core::serde::Deserialize)]
                struct #input_name {
                    #(pub #attributes),*
                }

                impl ash_core::Create<#action_name> for #ident {
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

                #[derive(::std::fmt::Debug, ash_core::serde::Deserialize)]
                struct #input_name {
                    #(pub #field_definitions),*
                }

                impl ash_core::Update<#action_name> for #ident {
                    type Input = #input_name;

                    fn apply_update_input(&mut self, input: Self::Input) {
                        #(#field_assignments)*
                        #(#change_ref_calls)*
                    }
                }
            }
        }
    });

    let name_segments = input.name.iter().map(|segment| segment.to_string());

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
        #[derive(::std::fmt::Debug, ::std::clone::Clone, ash_core::serde::Serialize, ash_core::serde::Deserialize)]
        struct #ident {
            #(#fields),*
        }

        impl ash_core::Resource for #ident {
            type PrimaryKey = #primary_key_type;

            // TODO: If the user specifies a different data layer for the resource, use that one instead.
            type DataLayer = ash_core::data_layer::in_memory::InMemoryDataLayer;

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
    use ash_extension_api::ResourceActionInput;
    use quote::quote;

    fn parse_resource(tokens: proc_macro2::TokenStream) -> ResourceMacroInput {
        syn::parse2::<ResourceMacroInput>(tokens).expect("failed to parse resource DSL")
    }

    #[test]
    fn minimal_resource_with_one_simple_attribute() {
        let input = parse_resource(quote! {
            name = Foo;

            attributes {
                id String;
            }
        });

        assert_eq!(input.name.len(), 1);
        assert_eq!(input.name[0], "Foo");

        assert_eq!(input.attributes.len(), 1);
        let attr = &input.attributes[0];
        assert_eq!(attr.name, "id");
        assert!(
            !attr.primary_key.value(),
            "primary_key should default to false"
        );
        assert!(!attr.generated.value(), "generated should default to false");
        assert!(attr.writable.value(), "writable should default to true");
        assert!(attr.default.is_none(), "default should be None");

        assert!(input.actions.is_empty());
    }

    #[test]
    fn dotted_name_parses_into_multiple_segments() {
        let input = parse_resource(quote! {
            name = Helpdesk.Support.Ticket;

            attributes {
                id String;
            }
        });

        assert_eq!(input.name.len(), 3);
        assert_eq!(input.name[0], "Helpdesk");
        assert_eq!(input.name[1], "Support");
        assert_eq!(input.name[2], "Ticket");
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

        assert_eq!(input.attributes.len(), 1);
        let attr = &input.attributes[0];
        assert_eq!(attr.name, "ticket_id");
        assert!(attr.primary_key.value());
        assert!(!attr.writable.value());
        assert!(attr.default.is_some());
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
        assert!(attr.generated.value());
        assert!(attr.primary_key.value());
        // writable should still be the default (true) since it wasn't set.
        assert!(attr.writable.value());
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

        assert_eq!(input.attributes.len(), 3);

        assert_eq!(input.attributes[0].name, "order_id");
        assert!(input.attributes[0].primary_key.value());
        assert!(!input.attributes[0].writable.value());

        assert_eq!(input.attributes[1].name, "item_name");
        assert!(!input.attributes[1].primary_key.value());
        assert!(input.attributes[1].writable.value());

        assert_eq!(input.attributes[2].name, "quantity");
        assert!(!input.attributes[2].primary_key.value());
        assert!(input.attributes[2].writable.value());
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

        assert_eq!(input.actions.len(), 1);
        assert_eq!(input.actions[0].name, "open");
        assert!(matches!(
            &input.actions[0].kind,
            ResourceActionInputKind::Create {
                accept: Accept::Default
            }
        ));
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

        assert_eq!(input.actions.len(), 1);
        assert_eq!(input.actions[0].name, "assign");
        match &input.actions[0].kind {
            ResourceActionInputKind::Create { accept } => match accept {
                Accept::Only(idents) => {
                    assert_eq!(idents.len(), 1);
                    assert_eq!(idents[0], "subject");
                }
                Accept::Default => panic!("expected Only accept, got Default"),
            },
            ResourceActionInputKind::Update(_) => {
                todo!()
            }
        }
    }

    #[test]
    fn no_actions_block_omitted() {
        let input = parse_resource(quote! {
            name = Simple;

            attributes {
                id u64;
            }
        });

        assert!(input.actions.is_empty());
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

        assert_eq!(input.name.len(), 3);
        assert_eq!(input.name[0], "Helpdesk");
        assert_eq!(input.name[1], "Support");
        assert_eq!(input.name[2], "Ticket");

        assert_eq!(input.attributes.len(), 3);

        let ticket_id = &input.attributes[0];
        assert_eq!(ticket_id.name, "ticket_id");
        assert!(ticket_id.primary_key.value());
        assert!(!ticket_id.writable.value());
        assert!(ticket_id.default.is_some());

        let subject = &input.attributes[1];
        assert_eq!(subject.name, "subject");
        assert!(!subject.primary_key.value());
        assert!(subject.writable.value());
        assert!(subject.default.is_none());

        let status = &input.attributes[2];
        assert_eq!(status.name, "status");
        assert!(!status.primary_key.value());
        assert!(status.writable.value());

        assert_eq!(input.actions.len(), 2);

        assert_eq!(input.actions[0].name, "open");
        assert!(matches!(
            &input.actions[0].kind,
            ResourceActionInputKind::Create {
                accept: Accept::Default
            }
        ));

        assert_eq!(input.actions[1].name, "assign");
        match &input.actions[1].kind {
            ResourceActionInputKind::Create { accept } => match accept {
                Accept::Only(idents) => {
                    assert_eq!(idents.len(), 1);
                    assert_eq!(idents[0], "subject");
                }
                Accept::Default => panic!("expected Only accept for assign action"),
            },
            ResourceActionInputKind::Update(_) => {
                todo!()
            }
        }
    }

    #[test]
    fn parse_simple_create_action() {
        let action = syn::parse2::<ResourceActionInput>(quote! {
            create open;
        })
        .expect("failed to parse action");

        assert_eq!(action.name, "open");
        assert!(matches!(
            action.kind,
            ResourceActionInputKind::Create {
                accept: Accept::Default
            }
        ));
    }

    #[test]
    fn parse_create_action_with_multiple_accept_idents() {
        let action = syn::parse2::<ResourceActionInput>(quote! {
            create bulk_insert {
                accept [name, email, age];
            }
        })
        .expect("failed to parse action");

        assert_eq!(action.name, "bulk_insert");
        match action.kind {
            ResourceActionInputKind::Create { accept } => match accept {
                Accept::Only(idents) => {
                    let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
                    assert_eq!(names, vec!["name", "email", "age"]);
                }
                Accept::Default => panic!("expected Only accept, got Default"),
            },
            ResourceActionInputKind::Update(_) => todo!(),
        }
    }

    #[test]
    fn unknown_action_kind_produces_error() {
        let result = syn::parse2::<ResourceActionInput>(quote! {
            destroy foo;
        });

        let err = result.expect_err("expected parse error for unknown action kind");
        let msg = err.to_string();
        assert!(
            msg.contains("Unexpected action kind"),
            "error should mention 'Unexpected action kind', got: {msg}"
        );
        assert!(
            msg.contains("destroy"),
            "error should mention the invalid kind 'destroy', got: {msg}"
        );
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

        let err = result.expect_err("expected parse error for unknown attribute key");
        let msg = err.to_string();
        assert!(
            msg.contains("Unexpected attribute key"),
            "error should mention 'Unexpected attribute key', got: {msg}"
        );
        assert!(
            msg.contains("bogus"),
            "error should mention the invalid key 'bogus', got: {msg}"
        );
    }

    #[test]
    fn missing_semicolon_after_name_produces_error() {
        let result = syn::parse2::<ResourceMacroInput>(quote! {
            name = Foo

            attributes {
                id String;
            }
        });

        assert!(
            result.is_err(),
            "expected parse error when semicolon is missing after name"
        );
    }

    #[test]
    fn parse_simple_update_action_with_default_accept() {
        let action = syn::parse2::<ResourceActionInput>(quote! {
            update close;
        })
        .expect("failed to parse update action");

        assert_eq!(action.name, "close");
        match &action.kind {
            ResourceActionInputKind::Update(update) => {
                assert!(
                    matches!(update.accept, Accept::Default),
                    "expected default accept"
                );
                assert!(update.changes.is_empty());
            }
            _ => panic!("expected Update action kind"),
        }
    }

    #[test]
    fn parse_update_action_with_accept_and_change_ref() {
        let action = syn::parse2::<ResourceActionInput>(quote! {
            update close {
                accept [];
                change_ref |resource| {
                    resource.status = TicketStatus::Closed;
                };
            }
        })
        .expect("failed to parse update action with change_ref");

        assert_eq!(action.name, "close");
        match &action.kind {
            ResourceActionInputKind::Update(update) => {
                match &update.accept {
                    Accept::Only(idents) => {
                        assert!(idents.is_empty(), "expected empty accept list")
                    }
                    Accept::Default => panic!("expected Only accept, got Default"),
                }
                assert_eq!(update.changes.len(), 1);
                assert!(
                    matches!(update.changes[0], UpdateChange::ChangeRef(_)),
                    "expected ChangeRef variant"
                );
            }
            _ => panic!("expected Update action kind"),
        }
    }

    #[test]
    fn parse_update_action_with_accept_fields() {
        let action = syn::parse2::<ResourceActionInput>(quote! {
            update reassign {
                accept [subject, status];
            }
        })
        .expect("failed to parse update action with accept fields");

        assert_eq!(action.name, "reassign");
        match &action.kind {
            ResourceActionInputKind::Update(update) => {
                match &update.accept {
                    Accept::Only(idents) => {
                        let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
                        assert_eq!(names, vec!["subject", "status"]);
                    }
                    Accept::Default => panic!("expected Only accept, got Default"),
                }
                assert!(update.changes.is_empty());
            }
            _ => panic!("expected Update action kind"),
        }
    }
}
