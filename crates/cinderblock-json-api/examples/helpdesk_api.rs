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
        read all {
            argument {
                status: TicketStatus,
            };

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

        destroy remove;
    }

    extensions {
        cinderblock_json_api {};
    }
}

#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    cinderblock_json_api::utoipa::ToSchema,
)]
enum TicketStatus {
    #[default]
    Open,
    Closed,
}

cinderblock_json_api::impl_field_schema!(TicketStatus);

#[tokio::main]
async fn main() -> cinderblock_core::Result<()> {
    // Initialize tracing so we can see the extension's log output.
    tracing_subscriber::fmt::init();

    let ctx = Context::new();

    // Seed some tickets so the list endpoint has data to return.
    cinderblock_core::create::<Ticket, Open>(
        OpenInput {
            subject: "The computer does not turn on".to_string(),
            status: TicketStatus::Open,
        },
        &ctx,
    )
    .await?;

    cinderblock_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "My mouse does not work".to_string(),
        },
        &ctx,
    )
    .await?;

    println!("Seeded 2 tickets");

    // Build the JSON API router — this auto-discovers all resources that
    // declared `cinderblock_json_api` in their extensions block.
    let router = cinderblock_json_api::router(ctx);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("Listening on http://0.0.0.0:3000");
    println!();
    println!("Try:");
    println!("  curl http://localhost:3000/helpdesk/support/ticket/all");
    println!("  curl http://localhost:3000/helpdesk/support/ticket/open-tickets");
    println!(
        "  curl -X POST http://localhost:3000/helpdesk/support/ticket/open -H 'Content-Type: application/json' -d '{{\"subject\": \"New ticket\", \"status\": \"Open\"}}'"
    );
    println!(
        "  curl -X PATCH http://localhost:3000/helpdesk/support/ticket/<id>/close -H 'Content-Type: application/json' -d '{{}}'"
    );
    println!("  curl -X DELETE http://localhost:3000/helpdesk/support/ticket/<id>/remove");
    println!();
    println!("OpenAPI spec:");
    println!("  curl http://localhost:3000/openapi.json");

    axum::serve(listener, router).await?;

    Ok(())
}
