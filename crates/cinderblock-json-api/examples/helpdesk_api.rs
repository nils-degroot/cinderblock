use cinderblock_core::{
    Context, resource,
    serde::{Deserialize, Serialize},
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// # Agent Resource
//
// Support agents who handle tickets. Declares a `has_many` relation to
// Ticket via the `agent_id` FK on the ticket side.
// ---------------------------------------------------------------------------

resource! {
    name = Helpdesk.Support.Agent;

    attributes {
        agent_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }

        name String;
    }

    relations {
        has_many tickets {
            ty Ticket;
            source_attribute agent_id;
        };
    }

    actions {
        read all_agents;

        // Returns each agent with their tickets nested inline.
        read all_agents_with_tickets {
            load [tickets];
        };

        create add_agent;
    }

    extensions {
        cinderblock_json_api {
            route = { method = GET; path = "/"; action = all_agents; };
            route = { method = GET; path = "/with-tickets"; action = all_agents_with_tickets; };
            route = { method = POST; path = "/"; action = add_agent; };
        };
    }
}

// ---------------------------------------------------------------------------
// # Ticket Resource
//
// Support tickets created by end users. Each ticket belongs to an agent
// via `agent_id`.
// ---------------------------------------------------------------------------

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
        agent_id Uuid;
    }

    relations {
        belongs_to agent {
            ty Agent;
            source_attribute agent_id;
        };
    }

    actions {
        read all {
            argument {
                status: TicketStatus,
            };

            filter { status == arg(status) };
        };

        // Returns every ticket with its assigned agent nested inline.
        read all_with_agent {
            load [agent];
        };

        create open;

        create assign {
            accept [subject, agent_id];
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
        cinderblock_json_api {
            route = { method = GET; path = "/"; action = all; };
            route = { method = GET; path = "/with-agent"; action = all_with_agent; };
            route = { method = POST; path = "/"; action = open; };
            route = { method = POST; path = "/assign"; action = assign; };
            route = { method = PATCH; path = "/{primary_key}"; action = close; };
            route = { method = DELETE; path = "/{primary_key}"; action = remove; };
        };
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

    // # Seed data
    //
    // Create two agents and assign tickets to them so the relation-loading
    // endpoints have data to return.

    let alice = cinderblock_core::create::<Agent, AddAgent>(
        AddAgentInput {
            name: "Alice".to_string(),
        },
        &ctx,
    )
    .await?;

    let bob = cinderblock_core::create::<Agent, AddAgent>(
        AddAgentInput {
            name: "Bob".to_string(),
        },
        &ctx,
    )
    .await?;

    cinderblock_core::create::<Ticket, Open>(
        OpenInput {
            subject: "The computer does not turn on".to_string(),
            status: TicketStatus::Open,
            agent_id: alice.agent_id,
        },
        &ctx,
    )
    .await?;

    cinderblock_core::create::<Ticket, Assign>(
        AssignInput {
            subject: "My mouse does not work".to_string(),
            agent_id: alice.agent_id,
        },
        &ctx,
    )
    .await?;

    cinderblock_core::create::<Ticket, Open>(
        OpenInput {
            subject: "VPN keeps disconnecting".to_string(),
            status: TicketStatus::Open,
            agent_id: bob.agent_id,
        },
        &ctx,
    )
    .await?;

    println!("Seeded 2 agents and 3 tickets");

    // Build the JSON API router — this auto-discovers all resources that
    // declared `cinderblock_json_api` in their extensions block.
    let router = cinderblock_json_api::router(ctx);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("Listening on http://0.0.0.0:3000");
    println!();
    println!("Try:");
    println!();
    println!("  # List agents (plain)");
    println!("  curl http://localhost:3000/helpdesk/support/agent/");
    println!();
    println!("  # List agents with their tickets loaded (has_many)");
    println!("  curl http://localhost:3000/helpdesk/support/agent/with-tickets");
    println!();
    println!("  # List tickets filtered by status");
    println!("  curl http://localhost:3000/helpdesk/support/ticket/?status=Open");
    println!();
    println!("  # List tickets with their agent loaded (belongs_to)");
    println!("  curl http://localhost:3000/helpdesk/support/ticket/with-agent");
    println!();
    println!("  # Create a ticket");
    println!("  curl -X POST http://localhost:3000/helpdesk/support/ticket/ \\");
    println!("    -H 'Content-Type: application/json' \\");
    println!(
        "    -d '{{\"subject\": \"New ticket\", \"status\": \"Open\", \"agent_id\": \"{}\"}}'",
        alice.agent_id,
    );
    println!();
    println!("OpenAPI spec:");
    println!("  curl http://localhost:3000/openapi.json");

    axum::serve(listener, router).await?;

    Ok(())
}
