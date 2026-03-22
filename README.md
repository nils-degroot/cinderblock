# Cinderblock

A declarative, resource-oriented application framework for Rust.

Define your domain model once using the `resource!` macro and Cinderblock generates the struct, CRUD operations, persistence layer, and REST API endpoints.

## Quick Example

```rust
use cinderblock_core::{Context, resource, serde::{Deserialize, Serialize}};
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
        create assign { accept [subject]; };
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
        cinderblock_sqlx { table = "tickets"; };
    }
}
```

This single declaration generates:

- A `Ticket` struct with serialization
- Typed action markers (`Open`, `Assign`, `Close`, `Remove`) with dedicated input structs
- `Create`, `Update`, and `Destroy` trait implementations
- REST API endpoints via Axum with OpenAPI documentation
- SQL persistence via SQLx with parameterized queries

## Key Concepts

### Named Actions

Actions are not generic CRUD verbs -- they are domain-specific operations with their own types and input validation:

```rust
cinderblock_core::create::<Ticket, Open>(open_input, &ctx).await?;
cinderblock_core::update::<Ticket, Close>(&ticket_id, close_input, &ctx).await?;
cinderblock_core::destroy::<Ticket, Remove>(&ticket_id, &ctx).await?;
```

Actions can restrict which fields they accept (`accept [field1, field2]`) and apply programmatic mutations via `change_ref` closures.

### Pluggable Data Layers

- **InMemoryDataLayer** (default) -- global `RwLock<HashMap>` for prototyping
- **SqliteDataLayer** (via `cinderblock-sqlx`) -- full SQLite persistence using SQLx

### Extension System

Extensions hook into the `resource!` macro to generate additional code:

```rust
extensions {
    cinderblock_json_api {};                    // REST API with OpenAPI
    cinderblock_sqlx { table = "my_table"; };   // SQL persistence
}
```

## Crates

| Crate | Description |
|---|---|
| `cinderblock-core` | Resource trait, CRUD functions, Context, in-memory data layer |
| `cinderblock-core-macros` | The `resource!` proc macro |
| `cinderblock-extension-api` | Shared DSL parser types for extension authors |
| `cinderblock-json-api` | JSON REST API extension (Axum router, OpenAPI, Swagger UI) |
| `cinderblock-json-api-macros` | JSON API extension proc macro |
| `cinderblock-sqlx` | SQLx data layer extension (SQLite) |
| `cinderblock-sqlx-macros` | SQLx extension proc macro |

## Running the Examples

```sh
# Core CRUD with in-memory storage
cargo run --example helpdesk -p cinderblock-core

# JSON API server with Swagger UI
cargo run --example helpdesk_api -p cinderblock-json-api --features swagger-ui
```

The API example starts a server on `http://localhost:3000` with endpoints like:

```
GET    /helpdesk/support/ticket
POST   /helpdesk/support/ticket/open
PATCH  /helpdesk/support/ticket/{id}/close
DELETE /helpdesk/support/ticket/{id}/remove
GET    /openapi.json
```

## Status

Early development (0.1.0). The API is not stable.

## License

Licensed under [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0).
