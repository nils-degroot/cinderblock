use ash_core::{
    Context, resource,
    serde::{Deserialize, Serialize},
};

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

#[derive(Debug, Default, Serialize, Deserialize)]
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

#[tokio::main]
async fn main() {
    let ctx = Context::new("helpdesk")
        .await
        .expect("Failed to setup context");

    let resource = ash_core::create::<Ticket, Open>(
        OpenInput {
            subject: "Help me!".to_string(),
            status: TicketStatus::Open,
        },
        &ctx,
    )
    .await
    .expect("Failed to open ticket");

    println!("Created a new ticket: {resource:?}");

    let resource = ash_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "Help me!".to_string(),
        },
        &ctx,
    )
    .await
    .expect("Failed to assign ticket");

    println!("Created a new ticket using assign: {resource:?}");
}
