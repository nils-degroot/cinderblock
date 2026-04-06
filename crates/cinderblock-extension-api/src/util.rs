use syn::Type;

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
