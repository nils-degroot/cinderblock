use core::iter::Iterator;

use cinderblock_extension_api::{
    ResourceActionInputKind, ResourceAttributeInput, ResourceMacroInput,
};

use crate::action::{
    ActionGenerateContext, create::generate_create, destroy::generate_destroy, read::generate_read,
    update::generate_update,
};

mod action;

#[cfg(test)]
mod tests;

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

    let primary_key = match input.primary_keys().collect::<Vec<_>>().as_slice() {
        [] => todo!("Support no primary keys"),
        [pk] => *pk,
        [_, ..] => todo!("Support multiple primary keys"),
    };

    let actions = input.actions.iter().map(|action| {
        let action_name = action.action_name.clone();

        let action_ctx = ActionGenerateContext {
            resource_name: ident.clone(),
            action: action.clone(),
            action_name: action_name.clone(),
            input: input.clone(),
        };

        match &action.kind {
            ResourceActionInputKind::Read(read) => generate_read(&action_ctx, read),
            ResourceActionInputKind::Create(create) => generate_create(&action_ctx, create),
            ResourceActionInputKind::Update(update) => generate_update(&action_ctx, update),
            ResourceActionInputKind::Destroy => generate_destroy(&action_ctx),
        }
    });

    // # Data layer selection
    //
    // If the user specified `data_layer = some::Path;` in the DSL, use that
    // path. Otherwise default to the built-in in-memory data layer.
    let data_layer_path = input.data_layer.clone().map_or_else(
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

    let before_create_override = input.before_create.clone().map(|closure| {
        let param = closure
            .inputs
            .first()
            .expect("before_create closure must have exactly one parameter");
        let body = &closure.body;
        quote::quote! {
            fn before_create(&mut self) {
                let hook = |#param: &mut Self| #body;
                hook(self);
            }
        }
    });

    let before_update_override = input.before_update.clone().map(|closure| {
        let param = closure
            .inputs
            .first()
            .expect("before_update closure must have exactly one parameter");
        let body = &closure.body;
        quote::quote! {
            fn before_update(&mut self) {
                let hook = |#param: &mut Self| #body;
                hook(self);
            }
        }
    });

    let primary_key_type = primary_key.ty.clone();
    let primary_key_generated = primary_key.generated.clone();
    let primary_key_value = {
        let name = primary_key.name.clone();
        quote::quote! { &self.#name }
    };

    let name_segments = input.name.str_segments();
    let resource_name_literal = input.name.as_literal();

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

            #before_create_override

            #before_update_override
        }

        #(#actions)*

        #(#extension_calls)*
    }
    .into()
}
