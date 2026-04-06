use super::*;
use assert2::{assert, check};
use quote::quote;

fn parse_resource(tokens: proc_macro2::TokenStream) -> ResourceMacroInput {
    let result = syn::parse2::<ResourceMacroInput>(tokens);
    assert!(let Ok(input) = result);
    input
}

#[test]
fn minimal_resource_with_one_simple_attribute() {
    let input = parse_resource(quote! {
        name = Foo;

        attributes {
            id String;
        }
    });

    check!(input.name.len() == 1);
    check!(input.name[0] == "Foo");

    check!(input.attributes.len() == 1);
    let attr = &input.attributes[0];
    check!(attr.name == "id");
    check!(!attr.primary_key.value());
    check!(!attr.generated.value());
    check!(attr.writable.value());
    check!(attr.default.is_none());

    check!(input.actions.is_empty());
    check!(input.extensions.is_empty());
    check!(input.data_layer.is_none());
}

#[test]
fn data_layer_keyword_parses_path() {
    let input = parse_resource(quote! {
        name = Ticket;
        data_layer = cinderblock_sqlx::sqlite::SqliteDataLayer;

        attributes {
            id String;
        }
    });

    assert!(let Some(path) = &input.data_layer);
    let path_str = quote::quote! { #path }.to_string();
    check!(path_str.contains("SqliteDataLayer"));
}

#[test]
fn data_layer_keyword_is_optional() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
        }
    });

    check!(input.data_layer.is_none());
}

#[test]
fn attribute_with_options_block() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            ticket_id Uuid {
                primary_key true;
                writable false;
                default || uuid::Uuid::new_v4();
            }
        }
    });

    check!(input.attributes.len() == 1);
    let attr = &input.attributes[0];
    check!(attr.name == "ticket_id");
    check!(attr.primary_key.value());
    check!(!attr.writable.value());
    check!(attr.default.is_some());
}

#[test]
fn attribute_with_generated_flag() {
    let input = parse_resource(quote! {
        name = Item;

        attributes {
            item_id Uuid {
                primary_key true;
                generated true;
            }
        }
    });

    let attr = &input.attributes[0];
    check!(attr.generated.value());
    check!(attr.primary_key.value());
    check!(attr.writable.value());
}

#[test]
fn multiple_attributes_mixed_simple_and_complex() {
    let input = parse_resource(quote! {
        name = Order;

        attributes {
            order_id Uuid {
                primary_key true;
                writable false;
            }
            item_name String;
            quantity u32;
        }
    });

    check!(input.attributes.len() == 3);

    check!(input.attributes[0].name == "order_id");
    check!(input.attributes[0].primary_key.value());
    check!(!input.attributes[0].writable.value());

    check!(input.attributes[1].name == "item_name");
    check!(!input.attributes[1].primary_key.value());
    check!(input.attributes[1].writable.value());

    check!(input.attributes[2].name == "quantity");
    check!(!input.attributes[2].primary_key.value());
    check!(input.attributes[2].writable.value());
}

#[test]
fn actions_block_with_simple_create() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
        }

        actions {
            create open;
        }
    });

    check!(input.actions.len() == 1);
    check!(input.actions[0].raw_name == "open");
    check!(let ResourceActionInputKind::Create(_) = &input.actions[0].kind);
}

#[test]
fn action_with_accept_list() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
        }

        actions {
            create assign {
                accept [subject];
            };
        }
    });

    check!(input.actions.len() == 1);
    check!(input.actions[0].raw_name == "assign");
    assert!(let ResourceActionInputKind::Create(ActionCreate{ accept: Accept::Only(idents) }) = &input.actions[0].kind);
    check!(idents.len() == 1);
    check!(idents[0] == "subject");
}

#[test]
fn no_actions_block_omitted() {
    let input = parse_resource(quote! {
        name = Simple;

        attributes {
            id u64;
        }
    });

    check!(input.actions.is_empty());
}

#[test]
fn full_helpdesk_example() {
    let input = parse_resource(quote! {
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

            create assign {
                accept [subject];
            };
        }
    });

    check!(input.name.len() == 3);
    check!(input.name[0] == "Helpdesk");
    check!(input.name[1] == "Support");
    check!(input.name[2] == "Ticket");

    check!(input.attributes.len() == 3);

    let ticket_id = &input.attributes[0];
    check!(ticket_id.name == "ticket_id");
    check!(ticket_id.primary_key.value());
    check!(!ticket_id.writable.value());
    check!(ticket_id.default.is_some());

    let subject = &input.attributes[1];
    check!(subject.name == "subject");
    check!(!subject.primary_key.value());
    check!(subject.writable.value());
    check!(subject.default.is_none());

    let status = &input.attributes[2];
    check!(status.name == "status");
    check!(!status.primary_key.value());
    check!(status.writable.value());

    check!(input.actions.len() == 2);

    check!(input.actions[0].raw_name == "open");
    assert!(let ResourceActionInputKind::Create(ActionCreate{ accept: Accept::Default }) = &input.actions[0].kind);

    check!(input.actions[1].raw_name == "assign");
    assert!(let ResourceActionInputKind::Create(ActionCreate{ accept: Accept::Only(idents) }) = &input.actions[1].kind);
    check!(idents.len() == 1);
    check!(idents[0] == "subject");
}

#[test]
fn parse_simple_create_action() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        create open;
    }));

    check!(action.raw_name == "open");
    assert!(let ResourceActionInputKind::Create(ActionCreate{ accept: Accept::Default }) = &action.kind);
}

#[test]
fn parse_create_action_with_multiple_accept_idents() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        create bulk_insert {
            accept [name, email, age];
        }
    }));

    check!(action.raw_name == "bulk_insert");
    assert!(let ResourceActionInputKind::Create(ActionCreate{ accept: Accept::Only(idents) }) = &action.kind);
    let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
    check!(names == vec!["name", "email", "age"]);
}

#[test]
fn parse_simple_destroy_action() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        destroy foo;
    }));

    check!(action.raw_name == "foo");
    check!(let ResourceActionInputKind::Destroy = action.kind);
}

#[test]
fn unknown_action_kind_produces_error() {
    let result = syn::parse2::<ResourceActionInput>(quote! {
        frobnicate foo;
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("Unexpected action kind"));
    check!(msg.contains("frobnicate"));
}

#[test]
fn unknown_attribute_option_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Thing;

        attributes {
            id String {
                bogus true;
            }
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("Unexpected attribute key"));
    check!(msg.contains("bogus"));
}

#[test]
fn missing_semicolon_after_name_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Foo

        attributes {
            id String;
        }
    });

    check!(let Err(_) = result);
}

#[test]
fn parse_simple_update_action_with_default_accept() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        update close;
    }));

    check!(action.raw_name == "close");
    assert!(let ResourceActionInputKind::Update(update) = &action.kind);
    check!(let Accept::Default = update.accept);
    check!(update.changes.is_empty());
}

#[test]
fn parse_update_action_with_accept_and_change_ref() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        update close {
            accept [];
            change_ref |resource| {
                resource.status = TicketStatus::Closed;
            };
        }
    }));

    check!(action.raw_name == "close");
    assert!(let ResourceActionInputKind::Update(update) = &action.kind);
    assert!(let Accept::Only(idents) = &update.accept);
    check!(idents.is_empty());
    check!(update.changes.len() == 1);
    check!(let UpdateChange::ChangeRef(_) = &update.changes[0]);
}

#[test]
fn parse_update_action_with_accept_fields() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        update reassign {
            accept [subject, status];
        }
    }));

    check!(action.raw_name == "reassign");
    assert!(let ResourceActionInputKind::Update(update) = &action.kind);
    assert!(let Accept::Only(idents) = &update.accept);
    let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
    check!(names == vec!["subject", "status"]);
    check!(update.changes.is_empty());
}

#[test]
fn resource_with_destroy_action() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
        }

        actions {
            create open;
            destroy remove;
        }
    });

    check!(input.actions.len() == 2);
    check!(input.actions[1].raw_name == "remove");
    check!(let ResourceActionInputKind::Destroy = input.actions[1].kind);
}

// -----------------------------------------------------------------------
// parse_attribute tests
// -----------------------------------------------------------------------

#[test]
fn parse_attribute_bool() {
    let tokens = quote! { enabled = true; };
    assert!(let Ok((key, value)) = syn::parse::Parser::parse2(parse_attribute::<LitBool>, tokens));

    check!(key == "enabled");
    check!(value.value());
}

#[test]
fn parse_attribute_missing_semicolon() {
    let tokens = quote! { enabled = true };
    let result = syn::parse::Parser::parse2(parse_attribute::<LitBool>, tokens);

    check!(let Err(_) = result);
}

#[test]
fn parse_attribute_missing_equals() {
    let tokens = quote! { enabled true; };
    let result = syn::parse::Parser::parse2(parse_attribute::<LitBool>, tokens);

    check!(let Err(_) = result);
}

// -----------------------------------------------------------------------
// Extension-specific tests
// -----------------------------------------------------------------------

#[test]
fn resource_with_extensions_block() {
    let input = parse_resource(quote! {
        name = Helpdesk.Support.Ticket;

        attributes {
            id String;
        }

        extensions {
            cinderblock_json_api {
                list = true;
            };
        }
    });

    check!(input.extensions.len() == 1);
    assert!(let Some(last_segment) = input.extensions[0].path.segments.last());
    check!(last_segment.ident == "cinderblock_json_api");
    check!(!input.extensions[0].config_tokens.is_empty());
}

#[test]
fn resource_with_actions_and_extensions() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
        }

        actions {
            create open;
        }

        extensions {
            cinderblock_json_api {
                list = true;
            };
        }
    });

    check!(input.actions.len() == 1);
    check!(input.extensions.len() == 1);
}

#[test]
fn extensions_block_without_actions() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
        }

        extensions {
            cinderblock_json_api {
                list = true;
            };
        }
    });

    check!(input.actions.is_empty());
    check!(input.extensions.len() == 1);
}

#[test]
fn multiple_extensions() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
        }

        extensions {
            cinderblock_json_api {
                list = true;
            };
            cinderblock_graphql {
                queries = true;
            };
        }
    });

    check!(input.extensions.len() == 2);
}

/// Verifies that `ExtensionMacroInput<C>` correctly parses forwarded
/// resource tokens followed by a `config = { ... }` block.
#[test]
fn extension_macro_input_parses_forwarded_tokens() {
    /// Minimal config struct for testing.
    struct TestConfig {
        list: bool,
    }

    impl Parse for TestConfig {
        fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
            let (key, value) = parse_attribute::<LitBool>(input)?;
            check!(key == "list");
            Ok(TestConfig {
                list: value.value(),
            })
        }
    }

    assert!(let Ok(input) = syn::parse2::<ExtensionMacroInput<TestConfig>>(quote! {
        {
            name = Helpdesk.Support.Ticket;

            attributes {
                id String;
            }

            extensions {
                cinderblock_json_api {
                    list = true;
                };
            }
        }

        config = {
            list = true;
        }
    }));

    check!(input.resource.name.len() == 3);
    check!(input.config.list);
}

// -----------------------------------------------------------------------
// Read action argument + arg() filter tests
// -----------------------------------------------------------------------

#[test]
fn read_action_with_no_arguments_or_filters() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read all;
    }));

    check!(action.raw_name == "all");
    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.arguments.is_empty());
    check!(read.filters.is_empty());
}

#[test]
fn read_action_with_literal_filter_only() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read open_only {
            filter { archived == false };
        }
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.arguments.is_empty());
    check!(read.filters.len() == 1);
    check!(read.filters[0].field == "archived");
    check!(let ReadFilterValue::Literal(_) = &read.filters[0].value);
}

#[test]
fn read_action_with_arguments_and_arg_filter() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read by_status {
            argument { status: TicketStatus };
            filter { status == arg(status) };
        }
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.arguments.len() == 1);
    check!(read.arguments[0].name == "status");
    check!(read.filters.len() == 1);
    assert!(let ReadFilterValue::Arg(arg_name) = &read.filters[0].value);
    check!(*arg_name == "status");
}

#[test]
fn read_action_with_multiple_arguments() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read by_status_and_priority {
            argument { status: TicketStatus, search: Option<String> };
            filter { status == arg(status) };
            filter { archived == false };
        }
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.arguments.len() == 2);
    check!(read.arguments[0].name == "status");
    check!(read.arguments[1].name == "search");
    check!(read.filters.len() == 2);
    check!(let ReadFilterValue::Arg(_) = &read.filters[0].value);
    check!(let ReadFilterValue::Literal(_) = &read.filters[1].value);
}

#[test]
fn read_action_rejects_undeclared_arg_reference() {
    let result = syn::parse2::<ResourceActionInput>(quote! {
        read broken {
            filter { status == arg(status) };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("undeclared argument"));
    check!(msg.contains("status"));
}

#[test]
fn read_action_mixed_arg_and_literal_filters() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read filtered {
            argument { priority: u32 };
            filter { priority == arg(priority) };
            filter { archived == false };
        }
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.filters.len() == 2);
    assert!(let ReadFilterValue::Arg(name) = &read.filters[0].value);
    check!(*name == "priority");
    check!(let ReadFilterValue::Literal(_) = &read.filters[1].value);
}

// -----------------------------------------------------------------------
// Paged read action tests
// -----------------------------------------------------------------------

#[test]
fn read_action_with_paged() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read all {
            paged;
        }
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    assert!(let Some(paged) = &read.paged);
    check!(paged.default_per_page.is_none());
    check!(paged.max_per_page.is_none());
}

#[test]
fn read_action_with_paged_config() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read search {
            argument { status: Option<String> };
            filter { status == arg(status) };
            paged {
                default_per_page 50;
                max_per_page 200;
            };
        }
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.arguments.len() == 1);
    check!(read.filters.len() == 1);
    assert!(let Some(paged) = &read.paged);
    check!(paged.default_per_page == Some(50));
    check!(paged.max_per_page == Some(200));
}

#[test]
fn read_action_with_paged_partial_config() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read all {
            paged {
                max_per_page 500;
            };
        }
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    assert!(let Some(paged) = &read.paged);
    check!(paged.default_per_page.is_none());
    check!(paged.max_per_page == Some(500));
}

#[test]
fn read_action_without_paged() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read all;
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.paged.is_none());
}

#[test]
fn read_action_paged_invalid_key_produces_error() {
    let result = syn::parse2::<ResourceActionInput>(quote! {
        read all {
            paged {
                bogus 42;
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("Unexpected paged config key"));
    check!(msg.contains("bogus"));
}

// -----------------------------------------------------------------------
// Relation parsing tests
// -----------------------------------------------------------------------

#[test]
fn resource_with_belongs_to_relation() {
    let input = parse_resource(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
        }
    });

    check!(input.relations.len() == 1);
    let rel = &input.relations[0];
    check!(rel.name == "author");
    check!(rel.kind == RelationKind::BelongsTo);
    check!(rel.source_attribute == "author_id");
    check!(rel.destination_attribute.is_none());
}

#[test]
fn resource_with_has_many_relation() {
    let input = parse_resource(quote! {
        name = Post;

        attributes {
            id String;
        }

        relations {
            has_many comments {
                ty Comment;
                source_attribute post_id;
            };
        }
    });

    check!(input.relations.len() == 1);
    let rel = &input.relations[0];
    check!(rel.name == "comments");
    check!(rel.kind == RelationKind::HasMany);
    check!(rel.source_attribute == "post_id");
    check!(rel.destination_attribute.is_none());
}

#[test]
fn resource_with_multiple_relations() {
    let input = parse_resource(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
            post_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
            belongs_to post {
                ty Post;
                source_attribute post_id;
            };
        }
    });

    check!(input.relations.len() == 2);
    check!(input.relations[0].name == "author");
    check!(input.relations[0].kind == RelationKind::BelongsTo);
    check!(input.relations[1].name == "post");
    check!(input.relations[1].kind == RelationKind::BelongsTo);
}

#[test]
fn relation_with_destination_attribute() {
    let input = parse_resource(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
                destination_attribute user_id;
            };
        }
    });

    check!(input.relations.len() == 1);
    assert!(let Some(dest) = &input.relations[0].destination_attribute);
    check!(*dest == "user_id");
}

#[test]
fn relation_missing_ty_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Comment;

        attributes {
            id String;
        }

        relations {
            belongs_to author {
                source_attribute author_id;
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("missing required `ty`"));
}

#[test]
fn relation_missing_source_attribute_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Comment;

        attributes {
            id String;
        }

        relations {
            belongs_to author {
                ty User;
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("missing required `source_attribute`"));
}

#[test]
fn relation_unknown_key_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Comment;

        attributes {
            id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
                bogus foo;
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("Unexpected relation key"));
    check!(msg.contains("bogus"));
}

#[test]
fn unknown_relation_kind_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Comment;

        attributes {
            id String;
        }

        relations {
            many_to_many tags {
                ty Tag;
                source_attribute tag_id;
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("Unexpected relation kind"));
    check!(msg.contains("many_to_many"));
}

#[test]
fn resource_with_no_relations_block() {
    let input = parse_resource(quote! {
        name = Simple;

        attributes {
            id String;
        }
    });

    check!(input.relations.is_empty());
}

#[test]
fn resource_with_empty_relations_block() {
    let input = parse_resource(quote! {
        name = Simple;

        attributes {
            id String;
        }

        relations {}
    });

    check!(input.relations.is_empty());
}

// -----------------------------------------------------------------------
// Load keyword tests
// -----------------------------------------------------------------------

#[test]
fn read_action_with_load() {
    let input = parse_resource(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
        }

        actions {
            read all_with_author {
                load [author];
            };
        }
    });

    check!(input.actions.len() == 1);
    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.load.len() == 1);
    check!(read.load[0] == "author");
}

#[test]
fn read_action_with_multiple_loads() {
    let input = parse_resource(quote! {
        name = Post;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
            has_many comments {
                ty Comment;
                source_attribute post_id;
            };
        }

        actions {
            read full {
                load [author, comments];
            };
        }
    });

    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.load.len() == 2);
    check!(read.load[0] == "author");
    check!(read.load[1] == "comments");
}

#[test]
fn read_action_without_load_has_empty_load_vec() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read all;
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.load.is_empty());
}

#[test]
fn read_action_load_with_filters_and_paged() {
    let input = parse_resource(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
        }

        actions {
            read search_with_author {
                argument { status: Option<String> };
                filter { status == arg(status) };
                load [author];
                paged;
            };
        }
    });

    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.arguments.len() == 1);
    check!(read.filters.len() == 1);
    check!(read.load.len() == 1);
    check!(read.load[0] == "author");
    check!(read.paged.is_some());
}

#[test]
fn load_referencing_undeclared_relation_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Comment;

        attributes {
            id String;
        }

        actions {
            read with_author {
                load [author];
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("undeclared relation"));
    check!(msg.contains("author"));
}

#[test]
fn load_referencing_nonexistent_relation_with_relations_block_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
        }

        actions {
            read with_tags {
                load [tags];
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("undeclared relation"));
    check!(msg.contains("tags"));
}

#[test]
fn relations_block_works_with_actions_and_extensions() {
    let input = parse_resource(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
        }

        actions {
            read all;
            read all_with_author {
                load [author];
            };
        }

        extensions {
            cinderblock_json_api {
                list = true;
            };
        }
    });

    check!(input.relations.len() == 1);
    check!(input.actions.len() == 2);
    check!(input.extensions.len() == 1);

    // First read action has no loads
    assert!(let ResourceActionInputKind::Read(read_all) = &input.actions[0].kind);
    check!(read_all.load.is_empty());

    // Second read action loads author
    assert!(let ResourceActionInputKind::Read(read_with_author) = &input.actions[1].kind);
    check!(read_with_author.load.len() == 1);
    check!(read_with_author.load[0] == "author");
}

#[test]
fn read_action_get_with_load() {
    let input = parse_resource(quote! {
        name = Comment;

        attributes {
            id String;
            author_id String;
        }

        relations {
            belongs_to author {
                ty User;
                source_attribute author_id;
            };
        }

        actions {
            read one_with_author {
                get;
                load [author];
            };
        }
    });

    check!(input.actions.len() == 1);
    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.get);
    check!(read.load.len() == 1);
    check!(read.load[0] == "author");
}

// -----------------------------------------------------------------------
// Order keyword tests
// -----------------------------------------------------------------------

#[test]
fn read_action_with_single_order_desc() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
            created_at String;
        }

        actions {
            read recent {
                order { created_at desc; };
            };
        }
    });

    check!(input.actions.len() == 1);
    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.orders.len() == 1);
    check!(read.orders[0].field == "created_at");
    check!(let OrderDirection::Desc = read.orders[0].direction);
}

#[test]
fn read_action_with_single_order_asc() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
            title String;
        }

        actions {
            read alphabetical {
                order { title asc; };
            };
        }
    });

    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.orders.len() == 1);
    check!(read.orders[0].field == "title");
    check!(let OrderDirection::Asc = read.orders[0].direction);
}

#[test]
fn read_action_with_order_default_direction() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
            title String;
        }

        actions {
            read alphabetical {
                order { title; };
            };
        }
    });

    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.orders.len() == 1);
    check!(read.orders[0].field == "title");
    check!(let OrderDirection::Asc = read.orders[0].direction);
}

#[test]
fn read_action_with_compound_order() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
            priority String;
            title String;
        }

        actions {
            read sorted {
                order { priority desc; title asc; };
            };
        }
    });

    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.orders.len() == 2);
    check!(read.orders[0].field == "priority");
    check!(let OrderDirection::Desc = read.orders[0].direction);
    check!(read.orders[1].field == "title");
    check!(let OrderDirection::Asc = read.orders[1].direction);
}

#[test]
fn read_action_with_order_and_filter_and_paged() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            id String;
            title String;
            done bool;
        }

        actions {
            read open_sorted {
                filter { done == false };
                order { title asc; };
                paged;
            };
        }
    });

    assert!(let ResourceActionInputKind::Read(read) = &input.actions[0].kind);
    check!(read.filters.len() == 1);
    check!(read.orders.len() == 1);
    check!(read.paged.is_some());
}

#[test]
fn read_action_without_order_has_empty_orders() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        read all;
    }));

    assert!(let ResourceActionInputKind::Read(read) = &action.kind);
    check!(read.orders.is_empty());
}

#[test]
fn order_with_invalid_direction_produces_error() {
    let result = syn::parse2::<ResourceActionInput>(quote! {
        read broken {
            order { title backwards; };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("Unexpected order direction"));
    check!(msg.contains("backwards"));
}

#[test]
fn order_referencing_undeclared_attribute_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Ticket;

        attributes {
            id String;
        }

        actions {
            read sorted {
                order { nonexistent desc; };
            };
        }
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("undeclared attribute"));
    check!(msg.contains("nonexistent"));
}

// -----------------------------------------------------------------------
// uuid_primary_key shortcut tests
// -----------------------------------------------------------------------

#[test]
fn uuid_primary_key_shortcut() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            ticket_id uuid_primary_key;
        }
    });

    check!(input.attributes.len() == 1);
    let attr = &input.attributes[0];
    check!(attr.name == "ticket_id");
    check!(attr.primary_key.value());
    check!(!attr.writable.value());
    check!(attr.default.is_some());

    let ty = &attr.ty;
    let ty_str = quote::quote! { #ty }.to_string();
    check!(ty_str.contains("Uuid"));
}

#[test]
fn uuid_primary_key_with_other_attributes() {
    let input = parse_resource(quote! {
        name = Order;

        attributes {
            order_id uuid_primary_key;
            item_name String;
            quantity u32;
        }
    });

    check!(input.attributes.len() == 3);

    check!(input.attributes[0].name == "order_id");
    check!(input.attributes[0].primary_key.value());
    check!(!input.attributes[0].writable.value());

    check!(input.attributes[1].name == "item_name");
    check!(!input.attributes[1].primary_key.value());
    check!(input.attributes[1].writable.value());

    check!(input.attributes[2].name == "quantity");
    check!(!input.attributes[2].primary_key.value());
    check!(input.attributes[2].writable.value());
}

#[test]
fn uuid_primary_key_with_override_block() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            ticket_id uuid_primary_key {
                generated true;
            }
        }
    });

    let attr = &input.attributes[0];
    check!(attr.primary_key.value());
    check!(!attr.writable.value());
    check!(attr.generated.value());
    check!(attr.default.is_some());
}

#[test]
fn uuid_primary_key_override_writable() {
    let input = parse_resource(quote! {
        name = Ticket;

        attributes {
            ticket_id uuid_primary_key {
                writable true;
            }
        }
    });

    let attr = &input.attributes[0];
    check!(attr.writable.value());
    check!(attr.primary_key.value());
    check!(attr.default.is_some());
}

#[test]
fn parse_before_create_hook() {
    let input = parse_resource(quote! {
        name = Post;

        attributes {
            id String;
        }

        before_create |post| {
            post.id = String::from("generated");
        };
    });

    check!(input.before_create.is_some());
    check!(input.before_update.is_none());
}

#[test]
fn parse_before_update_hook() {
    let input = parse_resource(quote! {
        name = Post;

        attributes {
            id String;
        }

        before_update |post| {
            post.id = String::from("updated");
        };
    });

    check!(input.before_create.is_none());
    check!(input.before_update.is_some());
}

#[test]
fn parse_both_hooks() {
    let input = parse_resource(quote! {
        name = Post;

        attributes {
            id String;
        }

        before_create |post| {
            post.id = String::from("created");
        };

        before_update |post| {
            post.id = String::from("updated");
        };
    });

    check!(input.before_create.is_some());
    check!(input.before_update.is_some());
}

#[test]
fn hooks_work_alongside_actions() {
    let input = parse_resource(quote! {
        name = Post;

        attributes {
            id String;
        }

        before_create |post| {
            post.id = String::from("created");
        };

        actions {
            create publish;
            update edit;
        }

        before_update |post| {
            post.id = String::from("updated");
        };
    });

    check!(input.before_create.is_some());
    check!(input.before_update.is_some());
    check!(input.actions.len() == 2);
}

#[test]
fn duplicate_before_create_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Post;

        attributes {
            id String;
        }

        before_create |post| {
            post.id = String::from("first");
        };

        before_create |post| {
            post.id = String::from("second");
        };
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("before_create"));
    check!(msg.contains("once"));
}

#[test]
fn duplicate_before_update_produces_error() {
    let result = syn::parse2::<ResourceMacroInput>(quote! {
        name = Post;

        attributes {
            id String;
        }

        before_update |post| {
            post.id = String::from("first");
        };

        before_update |post| {
            post.id = String::from("second");
        };
    });

    assert!(let Err(err) = result);
    let msg = err.to_string();
    check!(msg.contains("before_update"));
    check!(msg.contains("once"));
}

#[test]
fn no_hooks_declared_defaults_to_none() {
    let input = parse_resource(quote! {
        name = Simple;

        attributes {
            id u64;
        }
    });

    check!(input.before_create.is_none());
    check!(input.before_update.is_none());
}
