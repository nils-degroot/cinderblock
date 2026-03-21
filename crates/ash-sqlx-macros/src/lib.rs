// # SQLx Extension Proc Macro
//
// This proc macro is invoked by the `resource!` macro when a resource declares
// `ash_sqlx` in its `extensions { ... }` block. It receives the full resource
// DSL tokens plus the extension-specific config, and generates an
// `impl SqlResource for <Resource>` block.
//
// # Config syntax
//
// ```text
// ash_sqlx { table = "tickets"; }
// ```
//
// The only required config key is `table`, which specifies the SQLite table
// name for the resource.
//
// # Generated code
//
// For a resource `Ticket` with attributes `ticket_id`, `subject`, `status`
// (where `ticket_id` is the primary key), the macro generates:
//
// ```rust,ignore
// impl ash_sqlx::SqlResource for Ticket {
//     const TABLE_NAME: &'static str = "tickets";
//     const COLUMN_NAMES: &'static [&'static str] = &["ticket_id", "subject", "status"];
//     const PRIMARY_KEY_COLUMN: &'static str = "ticket_id";
//
//     fn bind_insert(&self, builder: &mut ash_sqlx::sqlx::QueryBuilder<'_, ash_sqlx::sqlx::Sqlite>) {
//         let mut sep = builder.separated(", ");
//         sep.push_bind(self.ticket_id.clone());
//         sep.push_bind(self.subject.clone());
//         sep.push_bind(self.status.clone());
//     }
//
//     fn bind_update(&self, builder: &mut ash_sqlx::sqlx::QueryBuilder<'_, ash_sqlx::sqlx::Sqlite>) {
//         let mut sep = builder.separated(", ");
//         sep.push("subject = ").push_bind_unseparated(self.subject.clone());
//         sep.push("status = ").push_bind_unseparated(self.status.clone());
//     }
//
//     fn bind_primary_key(
//         pk: &<Self as ash_core::Resource>::PrimaryKey,
//         builder: &mut ash_sqlx::sqlx::QueryBuilder<'_, ash_sqlx::sqlx::Sqlite>,
//     ) {
//         builder.push_bind(pk.clone());
//     }
//
//     fn from_row(row: &ash_sqlx::sqlx::sqlite::SqliteRow) -> ash_core::Result<Self> {
//         use ash_sqlx::sqlx::Row;
//         Ok(Self {
//             ticket_id: row.try_get("ticket_id")
//                 .map_err(|e| format!("decode column `ticket_id`: {e}"))?,
//             subject: row.try_get("subject")
//                 .map_err(|e| format!("decode column `subject`: {e}"))?,
//             status: row.try_get("status")
//                 .map_err(|e| format!("decode column `status`: {e}"))?,
//         })
//     }
// }
// ```

use ash_extension_api::ExtensionMacroInput;
use syn::{parse::Parse, Ident, LitStr};

// ---------------------------------------------------------------------------
// # Config Parsing
// ---------------------------------------------------------------------------

/// Extension-specific configuration parsed from inside `config = { ... }`.
///
/// Currently only supports a single required key:
/// - `table = "table_name";` — the SQL table backing this resource
struct SqlxConfig {
    table: String,
}

impl Parse for SqlxConfig {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut table: Option<String> = None;

        while !input.is_empty() {
            let key: Ident = input.fork().parse()?;

            match key.to_string().as_str() {
                "table" => {
                    let (_, value) = ash_extension_api::parse_attribute::<LitStr>(input)?;
                    table = Some(value.value());
                }
                got => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unexpected ash_sqlx config key, got `{got}`"),
                    ));
                }
            }
        }

        let table = table.ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "ash_sqlx config requires `table = \"...\";`",
            )
        })?;

        Ok(SqlxConfig { table })
    }
}

// ---------------------------------------------------------------------------
// # Code Generation
// ---------------------------------------------------------------------------

#[proc_macro]
pub fn __resource_extension(item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(item as ExtensionMacroInput<SqlxConfig>);

    let resource = &input.resource;
    let config = &input.config;

    // Derive the resource struct name from the last segment of the dotted name.
    let ident = resource
        .name
        .last()
        .expect("resource name must have at least one segment");

    let table_name = &config.table;

    // # Column metadata
    //
    // Build the list of all column names (matching attribute names) and
    // identify the primary key column.
    let column_names: Vec<String> = resource
        .attributes
        .iter()
        .map(|attr| attr.name.to_string())
        .collect();

    let primary_key_attr = resource
        .attributes
        .iter()
        .find(|attr| attr.primary_key.value())
        .expect("resource must have a primary key attribute");

    let pk_column = primary_key_attr.name.to_string();

    // # bind_insert
    //
    // Generates a `sep.push_bind(self.<field>.clone())` call for every
    // attribute, in declaration order. The separated helper auto-inserts
    // `, ` between bind placeholders.
    let bind_insert_calls: Vec<_> = resource
        .attributes
        .iter()
        .map(|attr| {
            let field = &attr.name;
            quote::quote! {
                sep.push_bind(self.#field.clone());
            }
        })
        .collect();

    // # bind_update
    //
    // Generates `col = ?` pairs for every non-primary-key attribute. We
    // use `Separated::push("col = ")` followed by `.push_bind_unseparated()`
    // so each pair is comma-separated but the `= ?` stays glued to its
    // column name.
    let non_pk_attributes: Vec<_> = resource
        .attributes
        .iter()
        .filter(|attr| !attr.primary_key.value())
        .collect();

    let bind_update_calls: Vec<_> = non_pk_attributes
        .iter()
        .map(|attr| {
            let field = &attr.name;
            let col_eq = format!("{} = ", attr.name);
            quote::quote! {
                sep.push(#col_eq).push_bind_unseparated(self.#field.clone());
            }
        })
        .collect();

    // # from_row
    //
    // Generates a `row.try_get("column")` call for each attribute. We
    // map sqlx errors to our `ash_core::Result` error type with a
    // descriptive message including the column name.
    let from_row_fields: Vec<_> = resource
        .attributes
        .iter()
        .map(|attr| {
            let field = &attr.name;
            let col = attr.name.to_string();
            quote::quote! {
                #field: row.try_get(#col)
                    .map_err(|e| format!("decode column `{}`: {e}", #col))?,
            }
        })
        .collect();

    // # Primary key column name literals for the const
    let column_name_literals: Vec<_> = column_names.iter().map(|c| quote::quote! { #c }).collect();

    quote::quote! {
        impl ash_sqlx::SqlResource for #ident {
            const TABLE_NAME: &'static str = #table_name;
            const COLUMN_NAMES: &'static [&'static str] = &[#(#column_name_literals),*];
            const PRIMARY_KEY_COLUMN: &'static str = #pk_column;

            fn bind_insert(
                &self,
                builder: &mut ash_sqlx::sqlx::QueryBuilder<'_, ash_sqlx::sqlx::Sqlite>,
            ) {
                let mut sep = builder.separated(", ");
                #(#bind_insert_calls)*
            }

            fn bind_update(
                &self,
                builder: &mut ash_sqlx::sqlx::QueryBuilder<'_, ash_sqlx::sqlx::Sqlite>,
            ) {
                let mut sep = builder.separated(", ");
                #(#bind_update_calls)*
            }

            fn bind_primary_key(
                pk: &<Self as ash_core::Resource>::PrimaryKey,
                builder: &mut ash_sqlx::sqlx::QueryBuilder<'_, ash_sqlx::sqlx::Sqlite>,
            ) {
                builder.push_bind(pk.clone());
            }

            fn from_row(
                row: &ash_sqlx::sqlx::sqlite::SqliteRow,
            ) -> ash_core::Result<Self> {
                use ash_sqlx::sqlx::Row;
                Ok(Self {
                    #(#from_row_fields)*
                })
            }
        }
    }
    .into()
}

// ---------------------------------------------------------------------------
// # Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use syn::parse::Parse;

    use super::*;

    fn parse_config(tokens: proc_macro2::TokenStream) -> SqlxConfig {
        let result = syn::parse2::<SqlxConfig>(tokens);
        assert!(let Ok(config) = result);
        config
    }

    #[test]
    fn parse_table_config() {
        let config = parse_config(quote::quote! {
            table = "tickets";
        });

        check!(config.table == "tickets");
    }

    #[test]
    fn missing_table_produces_error() {
        let result = syn::parse2::<SqlxConfig>(quote::quote! {});

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("requires `table"));
    }

    #[test]
    fn unknown_config_key_produces_error() {
        let result = syn::parse2::<SqlxConfig>(quote::quote! {
            table = "tickets";
            bogus = "value";
        });

        assert!(let Err(err) = result);
        let msg = err.to_string();
        check!(msg.contains("unexpected ash_sqlx config key"));
        check!(msg.contains("bogus"));
    }

    #[test]
    fn table_with_schema_prefix() {
        let config = parse_config(quote::quote! {
            table = "helpdesk_tickets";
        });

        check!(config.table == "helpdesk_tickets");
    }

    // Verify the full extension input parses correctly — this uses the
    // same `ExtensionMacroInput<SqlxConfig>` that the proc macro does.
    #[test]
    fn parse_full_extension_input() {
        let tokens = quote::quote! {
            {
                name = Helpdesk.Support.Ticket;

                attributes {
                    ticket_id Uuid {
                        primary_key true;
                        writable false;
                        default || uuid::Uuid::new_v4();
                    }
                    subject String;
                    status TicketStatus;
                }

                actions {
                    create open;
                }
            }

            config = {
                table = "tickets";
            }
        };

        let result = syn::parse2::<ExtensionMacroInput<SqlxConfig>>(tokens);
        assert!(let Ok(input) = result);
        check!(input.config.table == "tickets");
        check!(input.resource.attributes.len() == 3);

        // Verify primary key detection.
        let pk = input
            .resource
            .attributes
            .iter()
            .find(|a| a.primary_key.value());
        assert!(let Some(pk_attr) = pk);
        check!(pk_attr.name == "ticket_id");
    }
}
