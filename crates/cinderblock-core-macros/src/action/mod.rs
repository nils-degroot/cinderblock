use cinderblock_extension_api::{ResourceActionInput, ResourceMacroInput};
use syn::Ident;

pub(crate) mod create;
pub(crate) mod destroy;
pub(crate) mod read;
pub(crate) mod update;

#[derive(Debug)]
pub(crate) struct ActionGenerateContext {
    pub(crate) resource_name: Ident,
    pub(crate) action_name: Ident,
    pub(crate) action: ResourceActionInput,
    pub(crate) input: ResourceMacroInput,
}
