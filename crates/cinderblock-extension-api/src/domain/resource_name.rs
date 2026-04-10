use std::{fmt::Display, ops::Deref};

use syn::{Ident, Token, parse::Parse, punctuated::Punctuated};

#[derive(Debug, Clone)]
pub struct ResourceName {
    segments: Vec<Ident>,
}

impl ResourceName {
    pub fn str_segments(&self) -> Vec<String> {
        self.segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    }

    pub fn trailing_segment(&self) -> &Ident {
        self.segments
            .last()
            .expect("resource name must have at least one segment")
    }
}

impl Deref for ResourceName {
    type Target = Vec<Ident>;

    fn deref(&self) -> &Self::Target {
        &self.segments
    }
}

impl Parse for ResourceName {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let segments = Punctuated::<Ident, Token![.]>::parse_separated_nonempty(input)?
            .into_pairs()
            .map(|v| v.into_value())
            .collect::<Vec<_>>();

        Ok(Self { segments })
    }
}

impl Display for ResourceName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.str_segments().join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_segment() {
        let result = syn::parse2::<ResourceName>(quote::quote! { Helpdesk });
        assert2::assert!(let Ok(name) = result);
        assert2::check!(name.segments.len() == 1);
        assert2::check!(name.segments[0] == "Helpdesk");
    }

    #[test]
    fn parse_seperate_segments() {
        let result = syn::parse2::<ResourceName>(quote::quote! { Helpdesk.Support.Ticket });
        assert2::assert!(let Ok(name) = result);
        assert2::check!(name.segments.len() == 3);
        assert2::check!(name.segments[0] == "Helpdesk");
        assert2::check!(name.segments[1] == "Support");
        assert2::check!(name.segments[2] == "Ticket");
    }
}
