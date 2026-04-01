//! Core crate for the cinderblock framework — a declarative, resource-oriented
//! application framework for Rust.
//!
//! This crate provides the [`resource!`] macro, the [`Resource`] trait, CRUD
//! operation traits ([`Create`], [`Update`], [`Destroy`], [`ReadAction`]), the
//! runtime [`Context`], and a built-in [`InMemoryDataLayer`](data_layer::in_memory::InMemoryDataLayer)
//! for prototyping.
//!
//! # The `resource!` macro
//!
//! The [`resource!`] macro is the primary entry point for defining domain
//! models. It accepts a declarative DSL and generates:
//!
//! - A **struct** with the declared attributes (derives `Serialize`,
//!   `Deserialize`, `Clone`, `Debug`).
//! - A [`Resource`] trait impl with primary key metadata and the configured
//!   data layer.
//! - For each action, a **marker struct** and the corresponding CRUD trait
//!   impl. Create and update actions also generate an **input struct**.
//! - Extension dispatch — each declared extension receives the full DSL
//!   tokens so it can generate its own code (e.g. route handlers, SQL
//!   queries).
//!
//! ## DSL reference
//!
//! ```rust,ignore
//! use cinderblock_core::resource;
//!
//! resource! {
//!     // A dotted name identifying the resource. The last segment becomes the
//!     // struct name; all segments are available at runtime via `Resource::NAME`.
//!     name = Helpdesk.Support.Ticket;
//!
//!     // Optional: override the data layer. Defaults to `InMemoryDataLayer`.
//!     // data_layer = cinderblock_sqlx::sqlite::SqliteDataLayer;
//!
//!     attributes {
//!         // Each attribute is `name Type` followed by either `;` or an options block.
//!         ticket_id Uuid {
//!             primary_key true;   // Marks this as the primary key (default: false).
//!             writable false;     // Excludes from create/update input structs (default: true).
//!             generated true;     // Indicates the PK is auto-generated (default: false).
//!             default || Uuid::new_v4();  // Closure producing a default value.
//!         }
//!
//!         // Simple form — writable, not a primary key, no default.
//!         subject String;
//!         status TicketStatus;
//!     }
//!
//!     actions {
//!         // ── Read actions ──
//!         //
//!         // A read action returns `Vec<Resource>`. It can optionally declare
//!         // arguments (typed query parameters) and filters.
//!
//!         // Minimal read — no filters, no arguments. Arguments type is `()`.
//!         read all;
//!
//!         // Read with a compile-time literal filter.
//!         read open_tickets {
//!             filter { status == TicketStatus::Open };
//!         };
//!
//!         // Read with a runtime argument bound to a filter.
//!         // Generates a `ByStatusArguments` struct with a `status` field.
//!         read by_status {
//!             argument { status: TicketStatus };
//!             filter { status == arg(status) };
//!         };
//!
//!         // Optional arguments use `Option<T>`. When `None`, the filter is
//!         // skipped entirely at runtime.
//!         read search {
//!             argument { status: Option<TicketStatus> };
//!             filter { status == arg(status) };
//!         };
//!
//!         // ── Create actions ──
//!         //
//!         // A create action generates an input struct from the resource's
//!         // writable attributes and a `Create<A>` impl that builds a new
//!         // resource instance.
//!
//!         // Accepts all writable attributes. Generates `OpenInput { subject, status }`.
//!         create open;
//!
//!         // Restrict which fields the input struct includes.
//!         // Generates `AssignInput { subject }`.
//!         create assign {
//!             accept [subject];
//!         };
//!
//!         // ── Update actions ──
//!         //
//!         // An update action fetches the resource by primary key, applies
//!         // changes, and persists the result. It generates an input struct
//!         // and an `Update<A>` impl.
//!
//!         // Accepts all writable attributes.
//!         update edit;
//!
//!         // Accept no fields from the caller, but apply a programmatic
//!         // mutation via `change_ref`. Multiple `change_ref` blocks are
//!         // applied in order.
//!         update close {
//!             accept [];
//!             change_ref |ticket| {
//!                 ticket.status = TicketStatus::Closed;
//!             };
//!         };
//!
//!         // ── Destroy actions ──
//!         //
//!         // A destroy action deletes the resource by primary key.
//!         destroy remove;
//!     }
//!
//!     // Optional: declare extensions. Each extension module receives the
//!     // full resource DSL and its own configuration block, then generates
//!     // additional code (e.g. route handlers, SQL queries).
//!     extensions {
//!         cinderblock_json_api {
//!             route = { method = GET; path = "/"; action = all; };
//!             route = { method = POST; path = "/"; action = open; };
//!         };
//!
//!         cinderblock_sqlx {
//!             table = "tickets";
//!         };
//!     }
//! }
//! ```
//!
//! ## Generated items
//!
//! For a resource named `Helpdesk.Support.Ticket` with actions `open`
//! (create), `close` (update), `open_tickets` (read), and `remove` (destroy),
//! the macro generates:
//!
//! | Generated item | Kind | Description |
//! |---|---|---|
//! | `Ticket` | struct | The resource struct with all declared attributes |
//! | `Open` | struct (marker) | Create action marker |
//! | `OpenInput` | struct | Input fields for the `open` create action |
//! | `Close` | struct (marker) | Update action marker |
//! | `CloseInput` | struct | Input fields for the `close` update action |
//! | `OpenTickets` | struct (marker) | Read action marker |
//! | `Remove` | struct (marker) | Destroy action marker |
//!
//! Action names are converted to `PascalCase` for the marker and input struct
//! names (e.g. `open_tickets` becomes `OpenTickets`, and its input struct
//! would be `OpenTicketsInput`).
//!
//! ## Using the generated types
//!
//! ```rust,ignore
//! use cinderblock_core::Context;
//!
//! let ctx = Context::new();
//!
//! // Create
//! let ticket = cinderblock_core::create::<Ticket, Open>(
//!     OpenInput { subject: "Printer is broken".into(), status: TicketStatus::Open },
//!     &ctx,
//! ).await?;
//!
//! // Read (with arguments)
//! let open = cinderblock_core::read::<Ticket, ByStatus>(
//!     &ctx,
//!     &ByStatusArguments { status: TicketStatus::Open },
//! ).await?;
//!
//! // Read (no arguments — pass `&()`)
//! let all_open = cinderblock_core::read::<Ticket, OpenTickets>(&ctx, &()).await?;
//!
//! // Update
//! let closed = cinderblock_core::update::<Ticket, Close>(
//!     &ticket.ticket_id,
//!     CloseInput {},
//!     &ctx,
//! ).await?;
//!
//! // Destroy
//! let removed = cinderblock_core::destroy::<Ticket, Remove>(
//!     &ticket.ticket_id,
//!     &ctx,
//! ).await?;
//! ```

use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

pub use cinderblock_core_macros::resource;
pub use serde;

use crate::data_layer::DataLayer;

pub mod data_layer;

pub type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = std::result::Result<T, E>;

#[derive(Debug, Default)]
pub struct Context {
    data_layers: HashMap<TypeId, Box<dyn Any + Sync + Send + 'static>>,
}

impl Context {
    /// Generate a new context to be used by cinderblock applications.
    ///
    /// # Data layers
    ///
    /// This methods adds a [`data_layer::in_memory::InMemoryDataLayer`] by default.
    pub fn new() -> Self {
        let mut this = Self::default();
        this.register_data_layer(data_layer::in_memory::InMemoryDataLayer::new());
        this
    }

    /// Register a data layer instance so resources can look it up at runtime.
    pub fn register_data_layer<DL: std::fmt::Debug + Send + Sync + 'static>(
        &mut self,
        data_layer: DL,
    ) {
        self.data_layers
            .insert(data_layer.type_id(), Box::new(data_layer));
    }

    fn get_data_layer<DL: 'static>(&self) -> &DL {
        self.data_layers
            .get(&TypeId::of::<DL>())
            .expect("Requested data layer was not registered")
            .downcast_ref()
            .expect("Could not downcast value stored in data layer")
    }
}

/// Marker trait for a resource.
pub trait Resource:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + Clone + 'static
{
    /// Primary key type of the resource. Usually the type of the id for the resource.
    type PrimaryKey: std::fmt::Display + serde::de::DeserializeOwned + Send + Sync;

    /// Data layer that the resource uses.
    type DataLayer: DataLayer<Self>;

    /// Name with namespace of the resource. Each part of the array is a segment in the name
    /// (i.e. MyApp.Blog.Post).
    const NAME: &'static [&'static str];

    /// Wether the primary key of the resource is generated
    const PRIMARY_KEY_GENERATED: bool;

    /// Mathos that returns the primary key of the resource
    fn primary_key(&self) -> &Self::PrimaryKey;
}

/// Marker trait showing indicating that a struct is a read action.
pub trait ReadAction {
    /// Resource returned when calling the action.
    type Output: Resource;

    /// Arguments used to get the resource. Could be used in filters.
    type Arguments: Sync;
}

/// Trait indicating that a [`DataLayer`] can perform [`ReadAction`] `A`.
pub trait PerformRead<A: ReadAction> {
    /// Perform the read action on the provided data layer.
    fn read(&self, args: &A::Arguments) -> impl Future<Output = Result<Vec<A::Output>>>;
}

/// Trait placed on a [`Resource`] specifying how to create the resource using action `A`.
pub trait Create<A>: Resource {
    /// Input used to create the resource.
    type Input;

    /// Create an instance of the resource using [`Self::Input`].
    fn from_create_input(input: Self::Input) -> Self;
}

/// Trait placed on a [`Resource`] specifying how to update a resource using action `A`.
pub trait Update<A>: Resource {
    /// Arguments to pass to [`Self::apply_update_input`].
    type Input;

    /// Update an instance of self using [`Self::Input`].
    fn apply_update_input(&mut self, input: Self::Input);
}

/// Marker trait for destroy actions.
pub trait Destroy<A>: Resource {}

/// Create resource `R` using action `A`.
pub async fn create<R, A>(input: R::Input, ctx: &Context) -> Result<R>
where
    R: Create<A>,
{
    let resource = R::from_create_input(input);
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.create(resource.clone()).await?;
    Ok(resource)
}

/// Update resource `R` using action `A`. First
/// fetches an instance of `R` using the primary key.
pub async fn update<R, A>(primary_key: &R::PrimaryKey, input: R::Input, ctx: &Context) -> Result<R>
where
    R: Update<A>,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    let mut resource = dl.read(primary_key).await?;
    resource.apply_update_input(input);
    dl.update(resource.clone()).await?;
    Ok(resource)
}

/// Read resource `R` using action `A`.
pub async fn read<R, A>(ctx: &Context, args: &A::Arguments) -> Result<Vec<R>>
where
    R: Resource,
    A: ReadAction<Output = R>,
    R::DataLayer: PerformRead<A>,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    PerformRead::<A>::read(dl, args).await
}

/// Destroy resource `R` using action `A`.
pub async fn destroy<R, A>(primary_key: &R::PrimaryKey, ctx: &Context) -> Result<R>
where
    R: Destroy<A>,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.destroy(primary_key).await
}
