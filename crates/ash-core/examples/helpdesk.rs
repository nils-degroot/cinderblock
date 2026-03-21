use ash_core::{
    Context, resource,
    serde::{Deserialize, Serialize},
};
use uuid::Uuid;

resource! {
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

        update close {
            accept [];

            change_ref |ticket| {
                ticket.status = TicketStatus::Closed;
            };
        };
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
enum TicketStatus {
    #[default]
    Open,
    Closed,
}

#[tokio::main]
async fn main() {
    let ctx = Context::new("helpdesk")
        .await
        .expect("Failed to setup context");

    ash_core::create::<Ticket, Open>(
        OpenInput {
            subject: "The computer does not turn on".to_string(),
            status: TicketStatus::Open,
        },
        &ctx,
    )
    .await
    .expect("Failed to open ticket");

    println!("Created a new ticket");

    ash_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "My mouse does not work".to_string(),
        },
        &ctx,
    )
    .await
    .expect("Failed to assign ticket");

    println!("Created a new ticket using assign");

    let tickets = ash_core::list::<Ticket>(&ctx)
        .await
        .expect("Failed to list tickets");

    println!("Found these tickets:\n");

    for ticket in &tickets {
        println!(
            "id: {}\n\tsubject: {}\n\tstatus: {:?}\n",
            ticket.ticket_id, ticket.subject, ticket.status
        )
    }

    // Close the first ticket using the update action.
    let first_ticket = &tickets[0];
    println!("Closing ticket: {}", first_ticket.ticket_id);

    let closed = ash_core::update::<Ticket, Close>(&first_ticket.ticket_id, CloseInput {}, &ctx)
        .await
        .expect("Failed to close ticket");

    println!("Ticket closed: {:?}\n", closed.status);

    let tickets = ash_core::list::<Ticket>(&ctx)
        .await
        .expect("Failed to list tickets");

    println!("Tickets after closing:\n");

    for ticket in &tickets {
        println!(
            "id: {}\n\tsubject: {}\n\tstatus: {:?}\n",
            ticket.ticket_id, ticket.subject, ticket.status
        )
    }
}
