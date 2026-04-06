use syn::{Type, parse::Parse};

/// Checks whether a `syn::Type` is `Option<T>`.
///
/// We inspect the outermost path segment for the identifier `Option`. This
/// handles both `Option<T>` and `std::option::Option<T>` (by checking the
/// last segment).
pub fn is_optional(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        type_path
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "Option")
    } else {
        false
    }
}

pub fn parse_if<T: Parse>(
    input: &syn::parse::ParseBuffer<'_>,
    fun: impl Fn(&T) -> bool,
) -> Option<T> {
    T::parse(&input.fork()).ok().filter(fun).inspect(|_| {
        // Drop T from the actual input
        let _ = T::parse(input);
    })
}

pub fn drop_trailing_semi(input: &syn::parse::ParseBuffer<'_>) {
    parse_if::<syn::Token![;]>(input, |_| true);
}
