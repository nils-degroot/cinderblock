use std::collections::HashSet;

use cinderblock_extension_api::{Accept, ActionUpdate, UpdateChange};
use syn::{Ident, spanned::Spanned};

use crate::action::ActionGenerateContext;

pub(crate) fn generate_update(
    ctx: &ActionGenerateContext,
    update: &ActionUpdate,
) -> proc_macro2::TokenStream {
    let action_name = ctx.action_name.clone();
    let resource_name = ctx.resource_name.clone();

    let input_name = Ident::new(&format!("{action_name}Input"), ctx.action.raw_name.span());

    let attributes = ctx
        .input
        .attributes
        .iter()
        .filter(|attr| attr.writable.value());

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

        impl cinderblock_core::Update<#action_name> for #resource_name {
            type Input = #input_name;

            fn apply_update_input(&mut self, input: Self::Input) {
                #(#field_assignments)*
                #(#change_ref_calls)*
            }
        }
    }
}
