use ash_core::{
    Context, resource,
    serde::{Deserialize, Serialize},
};

resource! {
    name = Helpdesk.Support.Ticket;

    attributes = {
        integer_primary_key id;

        attribute subject String;

        attribute status TicketStatus;
    }

    actions = {
        create open;

        create assign accept [ subject ];
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
enum TicketStatus {
    #[default]
    Open,
    Closed,
}

resource! {
    name = Helpdesk.Support.Representative;

    attributes = {
        integer_primary_key id;

        attribute name String;
    }
}

#[tokio::main]
async fn main() {
    let ctx = Context::new("helpdesk")
        .await
        .expect("Failed to setup context");

    ash_core::create::<Ticket, Open>(
        OpenInput {
            subject: "Help me!".to_string(),
            status: TicketStatus::Open,
        },
        &ctx,
    )
    .await
    .expect("Failed to open ticket");

    println!("Created a new ticket");

    ash_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "Help me!".to_string(),
        },
        &ctx,
    )
    .await
    .expect("Failed to assign ticket");

    println!("Created a new ticket using assign");

    let tickets = ash_core::list::<Ticket>(&ctx)
        .await
        .expect("Failed to list tickets");

    println!("Found these tickets: {tickets:?}");
}
