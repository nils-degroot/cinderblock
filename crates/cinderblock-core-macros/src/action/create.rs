use std::collections::{HashMap, HashSet};

use cinderblock_extension_api::{Accept, ActionCreate};
use syn::Ident;

use crate::action::ActionGenerateContext;

pub(crate) fn generate_create(
    ctx: &ActionGenerateContext,
    create: &ActionCreate,
) -> proc_macro2::TokenStream {
    let input_name = Ident::new(
        &format!("{}Input", ctx.action.action_name),
        ctx.action.raw_name.span(),
    );

    let attributes = ctx
        .input
        .attributes
        .iter()
        .filter(|attr| attr.writable.value());

    let (present, mut missing_names) = match &create.accept {
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

    ctx.input
        .attributes
        .iter()
        .filter(|attr| !attr.writable.value() || !present.contains_key(&attr.name.to_string()))
        .for_each(|attr| {
            missing_names.insert(attr.name.to_string(), attr);
        });

    let attributes = present.values().map(|attr| attr.to_field_definition());

    let missing_names = missing_names.values().map(|attr| attr.to_default());

    let present_names = present.values().map(|attr| attr.name.clone());

    let action_name = ctx.action.action_name.clone();
    let resource_name = ctx.resource_name.clone();

    quote::quote! {
        #[derive(::std::fmt::Debug)]
        pub struct #action_name;

        #[derive(::std::fmt::Debug, cinderblock_core::serde::Deserialize)]
        pub struct #input_name {
            #(pub #attributes),*
        }

        impl cinderblock_core::Create<#action_name> for #resource_name {
            type Input = #input_name;

            fn from_create_input(input: Self::Input) -> Self {
                #resource_name {
                    // Iterate over attributes in #action_name.
                    #(#present_names: input.#present_names,)*

                    // All types attributes not present in #action_name should use default
                    #(#missing_names),*
                }
            }
        }
    }
}
