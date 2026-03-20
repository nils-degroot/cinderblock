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
