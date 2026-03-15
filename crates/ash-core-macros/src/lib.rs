use core::iter::Iterator;
use std::collections::HashSet;

use syn::{Ident, Token, Type, bracketed, parse::Parse, punctuated::Punctuated};

#[proc_macro]
pub fn resource(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
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
            .filter(|attr| attr.kind == ResourceAttributeInputKind::PrimaryKey)
            .collect::<Vec<_>>();

        match fields.len() {
            0 => todo!(),
            1 => {
                let ty = fields[0].ty.clone();
                quote::quote! { #ty }
            }
            _ => {
                let tys = fields.iter().map(|attr| attr.ty.clone());
                quote::quote! {
                    ( #(#tys),* )
                }
            }
        }
    };

    let primary_key_value = {
        let fields = input
            .attributes
            .iter()
            .filter(|attr| attr.kind == ResourceAttributeInputKind::PrimaryKey)
            .collect::<Vec<_>>();

        match fields.len() {
            0 => todo!(),
            1 => {
                let ty = fields[0].name.clone();
                quote::quote! { &self.#ty }
            }
            _ => {
                let names = fields.iter().map(|attr| attr.name.clone());

                quote::quote! {
                    ( #(&self.#names),* )
                }
            }
        }
    };

    let actions = input.actions.iter().map(|action| match &action.kind {
        ResourceActionInputKind::Create { accept } => {
            let action_name = convert_case::ccase!(pascal, action.name.to_string());
            let action_name = Ident::new(&action_name, action.name.span());
            let input_name = Ident::new(&format!("{action_name}Input"), action.name.span());

            let attributes = input
                .attributes
                .iter()
                .filter(|attribute| attribute.kind == ResourceAttributeInputKind::Attribute);

            let (present, missing) = match accept {
                ActionCreateAccept::Default => (attributes.collect::<Vec<_>>(), vec![]),
                ActionCreateAccept::Only(idents) => {
                    let idents = idents
                        .iter()
                        .map(|ident| ident.to_string())
                        .collect::<HashSet<_>>();

                    attributes.fold((vec![], vec![]), |(mut present, mut missing), attr| {
                        if idents.contains(&attr.name.to_string()) {
                            present.push(attr);
                        } else {
                            missing.push(attr);
                        }
                        (present, missing)
                    })
                }
            };

            dbg!((&present, &missing));

            let attributes = present.iter().map(|attr| attr.to_field_definition());
            let present_names = present.iter().map(|attr| attr.name.clone());
            let missing_names = missing.iter().map(|attr| attr.name.clone());

            let pks = input
                .attributes
                .iter()
                .filter(|attr| attr.kind == ResourceAttributeInputKind::PrimaryKey)
                .map(|pk| pk.name.clone());

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
                            // TODO: Properly handle PK's here
                            #(#pks: ::std::default::Default::default(),)*

                            // Iterate over attributes in #action_name.
                            #(#present_names: input.#present_names,)*

                            // All types attributes not present in #action_name should use default
                            #(#missing_names: ::std::default::Default::default(),)*
                        }
                    }
                }
            }
        }
    });

    let name_segments = input.name.iter().map(|segment| segment.to_string());

    quote::quote! {
        #[derive(::std::fmt::Debug, ash_core::serde::Serialize, ash_core::serde::Deserialize)]
        struct #ident {
            #(#fields),*
        }

        impl ash_core::Resource for #ident {
            type PrimaryKey = #primary_key_type;

            // TODO: If the user specifies a different data layer for the resource, use that one instead.
            type DataLayer = ash_core::data_layer::file_storage::FileStorageDataLayer;

            const NAME: &'static [&'static str] = &[#(#name_segments),*];

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
    kind: ResourceAttributeInputKind,
    name: Ident,
    ty: Type,
}

impl ResourceAttributeInput {
    fn to_field_definition(&self) -> proc_macro2::TokenStream {
        let name = self.name.clone();
        let ty = self.ty.clone();

        quote::quote! {
            #name: #ty
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ResourceAttributeInputKind {
    PrimaryKey,
    Attribute,
}

impl Parse for ResourceAttributeInputKind {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;

        match key.to_string().as_str() {
            "primary_key" => Ok(Self::PrimaryKey),
            "attribute" => Ok(Self::Attribute),
            got => Err(syn::Error::new(
                key.span(),
                format!("Unexpected attribute kind, got `{got}`"),
            )),
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
        let _: Token![=] = input.parse()?;

        let content;
        syn::braced!(content in input);

        let mut attributes = vec![];

        while !content.is_empty() {
            attributes.push(ResourceAttributeInput {
                kind: content.parse()?,
                name: content.parse()?,
                ty: content.parse()?,
            });

            let _: Token![;] = content.parse()?;
        }

        let mut actions = vec![];

        if input.peek(Ident) {
            println!("Parsing actions");
            let _: Ident = input.parse()?; // `actions`
            let _: Token![=] = input.parse()?;

            let content;
            syn::braced!(content in input);

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
