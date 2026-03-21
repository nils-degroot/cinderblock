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

    extensions {
        ash_json_api {};
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
enum TicketStatus {
    #[default]
    Open,
    Closed,
}

#[tokio::main]
async fn main() -> ash_core::Result<()> {
    // Initialize tracing so we can see the extension's log output.
    tracing_subscriber::fmt::init();

    let ctx = Context::new("helpdesk_api").await?;

    // Seed some tickets so the list endpoint has data to return.
    ash_core::create::<Ticket, Open>(
        OpenInput {
            subject: "The computer does not turn on".to_string(),
            status: TicketStatus::Open,
        },
        &ctx,
    )
    .await?;

    ash_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "My mouse does not work".to_string(),
        },
        &ctx,
    )
    .await?;

    println!("Seeded 2 tickets");

    // Build the JSON API router — this auto-discovers all resources that
    // declared `ash_json_api` in their extensions block.
    let router = ash_json_api::router(ctx);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("Listening on http://0.0.0.0:3000");
    println!();
    println!("Try:");
    println!("  curl http://localhost:3000/helpdesk/support/ticket");
    println!("  curl -X POST http://localhost:3000/helpdesk/support/ticket/open -H 'Content-Type: application/json' -d '{{\"subject\": \"New ticket\", \"status\": \"Open\"}}'");
    println!("  curl -X PATCH http://localhost:3000/helpdesk/support/ticket/<id>/close -H 'Content-Type: application/json' -d '{{}}'");

    axum::serve(listener, router).await?;

    Ok(())
}
