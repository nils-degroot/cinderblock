use std::fmt::Display;

use cinderblock_core::{
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
        read open_tickets {
            filter { status == TicketStatus::Open };
        };

        read by_status {
            argument { status: TicketStatus };
            filter { status == arg(status) };
        };

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
enum TicketStatus {
    #[default]
    Open,
    Closed,
}

#[tokio::main]
async fn main() {
    let ctx = Context::new();

    cinderblock_core::create::<Ticket, Open>(
        OpenInput {
            subject: "The computer does not turn on".to_string(),
            status: TicketStatus::Open,
        },
        &ctx,
    )
    .await
    .expect("Failed to open ticket");

    println!("Created a new ticket");

    cinderblock_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "My mouse does not work".to_string(),
        },
        &ctx,
    )
    .await
    .expect("Failed to assign ticket");

    println!("Created a new ticket using assign");

    let tickets = cinderblock_core::read::<Ticket, OpenTickets>(&ctx, &())
        .await
        .expect("Failed to list the open tickets");

    println!("Found these tickets:\n");

    for ticket in &tickets {
        println!("{ticket}\n");
    }

    // Demonstrate runtime arguments — query by status.
    let closed_tickets = cinderblock_core::read::<Ticket, ByStatus>(
        &ctx,
        &ByStatusArguments {
            status: TicketStatus::Closed,
        },
    )
    .await
    .expect("Failed to list closed tickets");

    println!("Closed tickets: {}\n", closed_tickets.len());

    // Close the first ticket using the update action.
    let first_ticket = &tickets[0];
    println!("Closing ticket: {}", first_ticket.ticket_id);

    let closed =
        cinderblock_core::update::<Ticket, Close>(&first_ticket.ticket_id, CloseInput {}, &ctx)
            .await
            .expect("Failed to close ticket");

    println!("Ticket closed: {:?}\n", closed.status);

    let tickets = cinderblock_core::read::<Ticket, OpenTickets>(&ctx, &())
        .await
        .expect("Failed to list the open tickets");

    println!("Tickets after closing:\n");

    for ticket in &tickets {
        println!("{ticket}\n");
    }
}

impl Display for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "id: {}\n\tsubject: {}\n\tstatus: {:?}",
            self.ticket_id, self.subject, self.status
        )
    }
}
