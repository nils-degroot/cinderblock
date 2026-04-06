use crate::action::ActionGenerateContext;

pub(crate) fn generate_destroy(ctx: &ActionGenerateContext) -> proc_macro2::TokenStream {
    let action_name = ctx.action_name.clone();
    let resource_name = ctx.resource_name.clone();

    quote::quote! {
        #[derive(::std::fmt::Debug)]
        pub struct #action_name;

        impl cinderblock_core::Destroy<#action_name> for #resource_name {}
    }
}
