use core::iter::Iterator;
use std::collections::{HashMap, HashSet};

use syn::{
    ExprClosure, Ident, LitBool, Token, Type, braced, bracketed, parse::Parse,
    punctuated::Punctuated,
};

#[proc_macro]
pub fn resource(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(item as ResourceMacroInput);

    dbg!(&input);

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
                ActionCreateAccept::Default => (
                    attributes
                        .map(|attr| (attr.name.to_string(), attr))
                        .collect::<HashMap<_, _>>(),
                    HashMap::new(),
                ),
                ActionCreateAccept::Only(idents) => {
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

                #[derive(::std::fmt::Debug)]
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
    });

    let name_segments = input.name.iter().map(|segment| segment.to_string());

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
    }
    .into()
}

#[derive(Debug)]
struct ResourceMacroInput {
    name: Vec<Ident>,
    attributes: Vec<ResourceAttributeInput>,
    actions: Vec<ResourceActionInput>,
}

#[derive(Debug)]
struct ResourceAttributeInput {
    name: Ident,
    ty: Type,
    primary_key: LitBool,
    generated: LitBool,
    writable: LitBool,
    default: Option<ExprClosure>,
}

impl ResourceAttributeInput {
    fn to_field_definition(&self) -> proc_macro2::TokenStream {
        let name = self.name.clone();
        let ty = self.ty.clone();

        quote::quote! {
            #name: #ty
        }
    }

    fn to_default(&self) -> proc_macro2::TokenStream {
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

#[derive(Debug)]
struct ResourceActionInput {
    kind: ResourceActionInputKind,
    name: Ident,
}

impl Parse for ResourceActionInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let kind: Ident = input.parse()?;
        let name: Ident = input.parse()?;

        let kind = match kind.to_string().as_str() {
            "create" => {
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;

                    ResourceActionInputKind::Create {
                        accept: ActionCreateAccept::Default,
                    }
                } else {
                    // TODO: Verify this
                    let _: Ident = input.parse()?; // `accept`

                    let content;
                    bracketed!(content in input);

                    let mut idents: Vec<Ident> = vec![];

                    while !content.is_empty() {
                        idents.push(content.parse()?);
                    }

                    let _: Token![;] = input.parse()?;

                    ResourceActionInputKind::Create {
                        accept: ActionCreateAccept::Only(idents),
                    }
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

#[derive(Debug)]
enum ResourceActionInputKind {
    Create { accept: ActionCreateAccept },
}

#[derive(Debug)]
enum ActionCreateAccept {
    Default,
    Only(Vec<Ident>),
}

impl Parse for ResourceMacroInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let _: Ident = input.parse()?; // `name`
        let _: Token![=] = input.parse()?;

        let name = Punctuated::<Ident, Token![.]>::parse_separated_nonempty(input)?
            .into_pairs()
            .map(|v| v.into_value())
            .collect::<Vec<_>>();

        let _: Token![;] = input.parse()?;

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
                let name: Ident = attribute_content.parse()?; // `attribute`

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

        let mut actions = vec![];

        if input.peek(Ident) {
            println!("Parsing actions");
            let _: Ident = input.parse()?; // `actions`

            let content;
            braced!(content in input);

            while !content.is_empty() {
                actions.push(content.parse()?);
            }
        }

        Ok(ResourceMacroInput {
            name,
            attributes,
            actions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                accept: ActionCreateAccept::Default
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
                create assign accept [ subject ];
            }
        });

        assert_eq!(input.actions.len(), 1);
        assert_eq!(input.actions[0].name, "assign");
        match &input.actions[0].kind {
            ResourceActionInputKind::Create { accept } => match accept {
                ActionCreateAccept::Only(idents) => {
                    assert_eq!(idents.len(), 1);
                    assert_eq!(idents[0], "subject");
                }
                ActionCreateAccept::Default => panic!("expected Only accept, got Default"),
            },
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

                create assign accept [ subject ];
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
                accept: ActionCreateAccept::Default
            }
        ));

        assert_eq!(input.actions[1].name, "assign");
        match &input.actions[1].kind {
            ResourceActionInputKind::Create { accept } => match accept {
                ActionCreateAccept::Only(idents) => {
                    assert_eq!(idents.len(), 1);
                    assert_eq!(idents[0], "subject");
                }
                ActionCreateAccept::Default => panic!("expected Only accept for assign action"),
            },
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
                accept: ActionCreateAccept::Default
            }
        ));
    }

    #[test]
    fn parse_create_action_with_multiple_accept_idents() {
        let action = syn::parse2::<ResourceActionInput>(quote! {
            create bulk_insert accept [ name email age ];
        })
        .expect("failed to parse action");

        assert_eq!(action.name, "bulk_insert");
        match action.kind {
            ResourceActionInputKind::Create { accept } => match accept {
                ActionCreateAccept::Only(idents) => {
                    let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
                    assert_eq!(names, vec!["name", "email", "age"]);
                }
                ActionCreateAccept::Default => panic!("expected Only accept, got Default"),
            },
        }
    }

    #[test]
    fn unknown_action_kind_produces_error() {
        let result = syn::parse2::<ResourceActionInput>(quote! {
            update foo;
        });

        let err = result.expect_err("expected parse error for unknown action kind");
        let msg = err.to_string();
        assert!(
            msg.contains("Unexpected action kind"),
            "error should mention 'Unexpected action kind', got: {msg}"
        );
        assert!(
            msg.contains("update"),
            "error should mention the invalid kind 'update', got: {msg}"
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
}
