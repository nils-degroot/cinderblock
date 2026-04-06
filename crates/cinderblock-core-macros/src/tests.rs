use super::*;
use assert2::{assert, check};
use cinderblock_extension_api::{Accept, ActionCreate, ResourceActionInput, UpdateChange};
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
}

#[test]
fn dotted_name_parses_into_multiple_segments() {
    let input = parse_resource(quote! {
        name = Helpdesk.Support.Ticket;

        attributes {
            id String;
        }
    });

    check!(input.name.len() == 3);
    check!(input.name[0] == "Helpdesk");
    check!(input.name[1] == "Support");
    check!(input.name[2] == "Ticket");
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
    // writable should still be the default (true) since it wasn't set.
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
    assert!(let ResourceActionInputKind::Create(ActionCreate { accept: Accept::Only(idents) }) = &input.actions[0].kind);
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
    check!(let ResourceActionInputKind::Create(_) = &input.actions[0].kind);

    check!(input.actions[1].raw_name == "assign");
    assert!(let ResourceActionInputKind::Create(ActionCreate { accept: Accept::Only(idents) }) = &input.actions[1].kind);
    check!(idents.len() == 1);
    check!(idents[0] == "subject");
}

#[test]
fn parse_simple_create_action() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        create open;
    }));

    check!(action.raw_name == "open");
    check!(let ResourceActionInputKind::Create(_) = &action.kind);
}

#[test]
fn parse_create_action_with_multiple_accept_idents() {
    assert!(let Ok(action) = syn::parse2::<ResourceActionInput>(quote! {
        create bulk_insert {
            accept [name, email, age];
        }
    }));

    check!(action.raw_name == "bulk_insert");
    assert!(let ResourceActionInputKind::Create(ActionCreate { accept: Accept::Only(idents) }) = &action.kind);
    let names: Vec<String> = idents.iter().map(|i| i.to_string()).collect();
    check!(names == vec!["name", "email", "age"]);
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
