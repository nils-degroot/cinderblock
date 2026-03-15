use ash_core::{Context, resource};

resource! {
    name = Helpdesk.Support.Ticket;

    attributes = {
        primary_key id i32;

        attribute subject String;

        attribute status TicketStatus;
    }

    actions = {
        create open;

        create assign accept [ subject ];
    }
}

#[derive(Debug, Default)]
enum TicketStatus {
    #[default]
    Open,
    Closed,
}

resource! {
    name = Helpdesk.Support.Representative;

    attributes = {
        primary_key id i32;

        attribute name String;
    }
}

fn main() {
    let ctx = Context::default();

    let resource = ash_core::create::<Ticket, Open>(
        OpenInput {
            subject: "Help me!".to_string(),
            status: TicketStatus::Open,
        },
        &ctx,
    )
    .expect("Failed to open ticket");

    println!("Created a new ticket: {resource:?}");

    let resource = ash_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "Help me!".to_string(),
        },
        &ctx,
    )
    .expect("Failed to assign ticket");

    println!("Created a new ticket using assign: {resource:?}");
}
