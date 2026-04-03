//! SQL persistence extension for cinderblock resources using
//! [SQLx](https://crates.io/crates/sqlx).
//!
//! When a resource declares `cinderblock_sqlx` in its `extensions { ... }`
//! block, the [`resource!`](cinderblock_core::resource) macro generates a
//! [`SqlResource`] trait impl that provides table metadata, column bind
//! methods, and row decoding. At runtime, a backend-specific data layer
//! (e.g. [`SqliteDataLayer`](sqlite::SqliteDataLayer) or
//! [`PostgresDataLayer`](postgres::PostgresDataLayer)) uses these generated
//! methods to build and execute SQL queries.
//!
//! # Extension configuration
//!
//! Inside the `extensions` block of a [`resource!`](cinderblock_core::resource)
//! invocation:
//!
//! ```rust,ignore
//! extensions {
//!     cinderblock_sqlx {
//!         // Required: the SQL table name to store this resource in.
//!         table = "tickets";
//!     };
//! }
//! ```
//!
//! The only configuration is the `table` name. Column names are derived
//! automatically from attribute names using `snake_case`.
//!
//! # Database-generated columns
//!
//! Attributes marked `generated true` are omitted from INSERT and UPDATE
//! statements — their values come from the database (e.g. autoincrement
//! primary keys, server-side DEFAULT expressions). The column is still
//! read back via `RETURNING *` / SELECT so the Rust struct always has the
//! correct database-assigned value.
//!
//! ```rust,ignore
//! attributes {
//!     id i64 {
//!         primary_key true;
//!         generated true;     // Database provides the value via AUTOINCREMENT
//!         writable false;
//!     }
//!     title String;
//! }
//! ```
//!
//! # Type mapping
//!
//! This extension deliberately does **not** define its own type-mapping trait.
//! Instead, it relies on SQLx's existing [`sqlx::Type`], [`sqlx::Encode`], and
//! [`sqlx::Decode`] implementations. The generated [`SqlResource`] methods use
//! `QueryBuilder::push_bind()` (which requires `Encode + Type`) and
//! `Row::try_get()` (which requires `Decode + Type`).
//!
//! For custom types like enums, implement SQLx's traits directly — for
//! example via `#[derive(sqlx::Type)]` or a serde-based approach.
//!
//! # Read filters
//!
//! Read actions with `filter` clauses in the `resource!` DSL generate a
//! [`SqlReadAction`] impl. The generated `bind_filters` method dynamically
//! builds `WHERE` clauses:
//!
//! - **Literal filters** (`filter { status == TicketStatus::Open }`) always
//!   emit a `WHERE column = ?` clause.
//! - **Argument-bound filters** (`filter { status == arg(status) }`) bind the
//!   runtime argument value. When the argument type is `Option<T>` and the
//!   value is `None`, the clause is omitted entirely.
//!
//! # Setup
//!
//! Register a data layer on the [`Context`](cinderblock_core::Context):
//!
//! ```rust,ignore
//! use cinderblock_core::Context;
//! use cinderblock_sqlx::sqlite::SqliteDataLayer;
//!
//! let pool = sqlx::SqlitePool::connect("sqlite::memory:").await?;
//! let mut ctx = Context::new();
//! ctx.register_data_layer(SqliteDataLayer::new(pool));
//! ```
//!
//! Then set the resource's data layer in the `resource!` DSL:
//!
//! ```rust,ignore
//! resource! {
//!     name = Helpdesk.Support.Ticket;
//!     data_layer = cinderblock_sqlx::sqlite::SqliteDataLayer;
//!
//!     attributes {
//!         ticket_id Uuid {
//!             primary_key true;
//!             writable false;
//!             default || uuid::Uuid::new_v4();
//!         }
//!         subject String;
//!         status String;
//!     }
//!
//!     actions {
//!         read all;
//!         create open;
//!         update edit;
//!         destroy remove;
//!     }
//!
//!     extensions {
//!         cinderblock_sqlx {
//!             table = "tickets";
//!         };
//!     }
//! }
//! ```

use cinderblock_core::ReadAction;
pub use cinderblock_sqlx_macros::__resource_extension;
pub use sqlx;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

// ---------------------------------------------------------------------------
// # SqlResource Trait
// ---------------------------------------------------------------------------

/// Schema and row-conversion interface for resources stored in SQL.
///
/// This trait is generic over the sqlx `Database` backend so that a single
/// generated impl can work with both SQLite and PostgreSQL (or any other
/// sqlx-supported backend).
///
/// The generated methods lean on sqlx's native `Encode`/`Decode`/`Type`
/// impls rather than a framework-specific trait, so any type that sqlx
/// already knows how to handle (String, integers, bool, Uuid with the
/// `uuid` feature, etc.) just works.
pub trait SqlResource<DB: sqlx::Database>: cinderblock_core::Resource {
    const TABLE_NAME: &'static str;

    /// All column names on the table, in declaration order. Used for
    /// SELECT and from_row decoding.
    const COLUMN_NAMES: &'static [&'static str];

    /// Column names included in INSERT statements — excludes columns
    /// marked `generated true`, whose values are provided by the database
    /// (e.g. autoincrement primary keys, server-side defaults).
    const INSERT_COLUMN_NAMES: &'static [&'static str];

    const PRIMARY_KEY_COLUMN: &'static str;

    /// Push bind values for non-generated columns into the query builder,
    /// separated by commas (in the same order as `INSERT_COLUMN_NAMES`).
    ///
    /// Columns marked `generated true` are omitted — those values come
    /// from the database itself.
    ///
    /// The caller is responsible for emitting the `INSERT INTO ... VALUES (`
    /// prefix before calling this, and the closing `)` afterwards.
    fn bind_insert(&self, builder: &mut sqlx::QueryBuilder<'_, DB>);

    /// Push bind values for non-primary-key, non-generated columns into the
    /// query builder, each as `column_name = ?` separated by commas.
    ///
    /// The caller emits `UPDATE table SET ` before calling this, and
    /// ` WHERE pk = ?` afterwards.
    fn bind_update(&self, builder: &mut sqlx::QueryBuilder<'_, DB>);

    /// Push the primary key value as a bind parameter.
    ///
    /// This is a generated method so the concrete PK type's `Encode` impl
    /// is used directly, without needing sqlx bounds on the `SqlResource`
    /// trait definition itself.
    fn bind_primary_key(
        pk: &<Self as cinderblock_core::Resource>::PrimaryKey,
        builder: &mut sqlx::QueryBuilder<'_, DB>,
    );

    /// Decode all columns from a row and reconstruct the resource.
    fn from_row(row: &DB::Row) -> Result<Self, Box<dyn std::error::Error + Send + Sync>>;
}

pub trait SqlReadAction<DB: sqlx::Database>: ReadAction
where
    Self::Output: SqlResource<DB>,
{
    /// Conditionally push filter clauses (including the WHERE keyword) into
    /// the query builder. Returns `true` if any WHERE conditions were added.
    fn bind_filters(builder: &mut sqlx::QueryBuilder<'_, DB>, args: &Self::Arguments) -> bool;

    /// Push a static `ORDER BY` clause into the query builder.
    ///
    /// When the read action has no `order` clauses, this is a no-op.
    fn bind_order(builder: &mut sqlx::QueryBuilder<'_, DB>);
}

/// Same contract as [`SqlReadAction`] but for paged read actions.
///
/// A separate trait is used so the generated code can distinguish paged
/// vs non-paged read actions at compile time.
pub trait SqlPagedReadAction<DB: sqlx::Database>: ReadAction
where
    Self::Output: SqlResource<DB>,
    Self::Arguments: cinderblock_core::Paged,
{
    fn bind_filters(builder: &mut sqlx::QueryBuilder<'_, DB>, args: &Self::Arguments) -> bool;

    fn bind_order(builder: &mut sqlx::QueryBuilder<'_, DB>);
}

/// Unified execution trait that bridges the filter-based traits
/// ([`SqlReadAction`] and [`SqlPagedReadAction`]) to the framework's
/// [`PerformRead`](cinderblock_core::PerformRead) trait.
///
/// The `cinderblock_sqlx` extension macro generates explicit impls of this
/// trait for each read action, delegating to `SqlReadAction::execute` or
/// `SqlPagedReadAction::execute` as appropriate. A single blanket
/// `PerformRead` impl on the data layer then dispatches to this trait.
pub trait SqlPerformRead<DB: sqlx::Database>: ReadAction {
    fn execute(
        pool: &sqlx::Pool<DB>,
        args: &Self::Arguments,
    ) -> impl std::future::Future<Output = Result<Self::Response, cinderblock_core::ListError>> + Send;
}

/// Unified execution trait for get (single-resource) read actions.
///
/// Similar to [`SqlPerformRead`] but returns a single resource or
/// `ReadError::NotFound`. The `cinderblock_sqlx` extension macro generates
/// impls of this trait for get-actions. A blanket `PerformReadOne` impl on
/// the data layer dispatches to this trait.
pub trait SqlPerformReadOne<DB: sqlx::Database>: ReadAction {
    fn execute(
        pool: &sqlx::Pool<DB>,
        args: &Self::Arguments,
    ) -> impl std::future::Future<Output = Result<Self::Response, cinderblock_core::ReadError>> + Send;
}

// ---------------------------------------------------------------------------
// # Backend-Specific Query Execution
// ---------------------------------------------------------------------------

// sqlx's QueryBuilder<DB> has a drop-checker limitation with generic DB types
// that prevents `builder.build().fetch_*(pool)` from compiling when DB is a
// type parameter. We stamp out concrete functions per backend to avoid this.

macro_rules! impl_sql_query_functions {
    ($db:ty, $mod_name:ident) => {
        pub mod $mod_name {
            use super::*;

            pub async fn execute_sql_read<A>(
                pool: &sqlx::Pool<$db>,
                args: &A::Arguments,
            ) -> Result<Vec<A::Output>, cinderblock_core::ListError>
            where
                A: SqlReadAction<$db>,
                A::Output: SqlResource<$db>,
            {
                let mut builder = sqlx::QueryBuilder::<$db>::new(format!(
                    "SELECT * FROM {} ",
                    <A::Output as SqlResource<$db>>::TABLE_NAME,
                ));
                A::bind_filters(&mut builder, args);
                A::bind_order(&mut builder);

                let rows: Vec<<$db as sqlx::Database>::Row> =
                    builder.build().fetch_all(pool).await.map_err(|e| {
                        cinderblock_core::ListError::DataLayer(
                            format!(
                                "read from `{}`: {e}",
                                <A::Output as SqlResource<$db>>::TABLE_NAME,
                            )
                            .into(),
                        )
                    })?;

                let mut result = Vec::with_capacity(rows.len());
                for row in &rows {
                    result.push(
                        <A::Output as SqlResource<$db>>::from_row(row)
                            .map_err(cinderblock_core::ListError::DataLayer)?,
                    );
                }
                Ok(result)
            }

            pub async fn execute_sql_read_one<R>(
                pool: &sqlx::Pool<$db>,
                pk: &R::PrimaryKey,
            ) -> Result<R, cinderblock_core::ReadError>
            where
                R: SqlResource<$db> + cinderblock_core::Resource,
            {
                let mut builder = sqlx::QueryBuilder::<$db>::new(format!(
                    "SELECT * FROM {} WHERE {} = ",
                    R::TABLE_NAME,
                    R::PRIMARY_KEY_COLUMN,
                ));
                R::bind_primary_key(pk, &mut builder);

                let row: Option<<$db as sqlx::Database>::Row> =
                    builder.build().fetch_optional(pool).await.map_err(|e| {
                        cinderblock_core::ReadError::DataLayer(
                            format!("read from `{}`: {e}", R::TABLE_NAME).into(),
                        )
                    })?;

                match row {
                    Some(row) => R::from_row(&row).map_err(cinderblock_core::ReadError::DataLayer),
                    None => Err(cinderblock_core::ReadError::NotFound {
                        primary_key: pk.to_string(),
                    }),
                }
            }

            pub async fn execute_sql_paged_read<A>(
                pool: &sqlx::Pool<$db>,
                args: &A::Arguments,
            ) -> Result<cinderblock_core::PaginatedResult<A::Output>, cinderblock_core::ListError>
            where
                A: SqlPagedReadAction<$db>,
                A::Output: SqlResource<$db>,
                A::Arguments: cinderblock_core::Paged,
            {
                use cinderblock_core::{Paged, PaginatedResult, PaginationMeta};

                let table = <A::Output as SqlResource<$db>>::TABLE_NAME;

                let mut count_builder =
                    sqlx::QueryBuilder::<$db>::new(format!("SELECT COUNT(*) FROM {} ", table));
                A::bind_filters(&mut count_builder, args);

                let total: i64 = {
                    use sqlx::Row;
                    count_builder
                        .build()
                        .fetch_one(pool)
                        .await
                        .map_err(|e| {
                            cinderblock_core::ListError::DataLayer(
                                format!("count from `{table}`: {e}").into(),
                            )
                        })?
                        .try_get(0)
                        .map_err(|e| {
                            cinderblock_core::ListError::DataLayer(
                                format!("decode count from `{table}`: {e}").into(),
                            )
                        })?
                };
                let total = total as u64;

                let page = args.page();
                let per_page = args.per_page();
                let total_pages = total.div_ceil(per_page as u64) as u32;
                let offset = (page - 1) * per_page;

                let mut builder =
                    sqlx::QueryBuilder::<$db>::new(format!("SELECT * FROM {} ", table));
                A::bind_filters(&mut builder, args);
                A::bind_order(&mut builder);
                builder.push(format!(" LIMIT {} OFFSET {}", per_page, offset));

                let rows: Vec<<$db as sqlx::Database>::Row> =
                    builder.build().fetch_all(pool).await.map_err(|e| {
                        cinderblock_core::ListError::DataLayer(
                            format!("paged read from `{table}`: {e}").into(),
                        )
                    })?;

                let mut data = Vec::with_capacity(rows.len());
                for row in &rows {
                    data.push(
                        <A::Output as SqlResource<$db>>::from_row(row)
                            .map_err(cinderblock_core::ListError::DataLayer)?,
                    );
                }

                Ok(PaginatedResult {
                    data,
                    meta: PaginationMeta {
                        page,
                        per_page,
                        total,
                        total_pages,
                    },
                })
            }
        }
    };
}

#[cfg(feature = "sqlite")]
impl_sql_query_functions!(sqlx::Sqlite, sqlite_query);

#[cfg(feature = "postgres")]
impl_sql_query_functions!(sqlx::Postgres, postgres_query);

// ---------------------------------------------------------------------------
// # DataLayer macro
// ---------------------------------------------------------------------------

/// Implements `DataLayer`, `PerformRead`, and `PerformReadOne` for a
/// backend-specific data layer struct. Both `SqliteDataLayer` and
/// `PostgresDataLayer` invoke this macro with their respective sqlx types.
///
/// # Arguments
///
/// - `$dl`: The data layer struct name (e.g. `SqliteDataLayer`)
/// - `$db`: The sqlx database type (e.g. `sqlx::Sqlite`)
macro_rules! impl_sql_data_layer {
    ($dl:ty, $db:ty) => {
        impl<R> cinderblock_core::data_layer::DataLayer<R> for $dl
        where
            R: cinderblock_core::Resource + $crate::SqlResource<$db> + 'static,
        {
            async fn create(&self, resource: R) -> Result<R, cinderblock_core::CreateError> {
                let columns = R::INSERT_COLUMN_NAMES.join(", ");

                let mut builder = sqlx::QueryBuilder::new(format!(
                    "INSERT INTO {} ({}) VALUES (",
                    R::TABLE_NAME,
                    columns,
                ));

                resource.bind_insert(&mut builder);
                builder.push(") RETURNING *");

                let row: <$db as sqlx::Database>::Row =
                    builder.build().fetch_one(self.pool()).await.map_err(|e| {
                        cinderblock_core::CreateError::DataLayer(
                            format!("insert into `{}`: {e}", R::TABLE_NAME).into(),
                        )
                    })?;

                R::from_row(&row).map_err(cinderblock_core::CreateError::DataLayer)
            }

            async fn read(
                &self,
                primary_key: &R::PrimaryKey,
            ) -> Result<R, cinderblock_core::ReadError> {
                let mut builder = sqlx::QueryBuilder::new(format!(
                    "SELECT * FROM {} WHERE {} = ",
                    R::TABLE_NAME,
                    R::PRIMARY_KEY_COLUMN,
                ));

                R::bind_primary_key(primary_key, &mut builder);

                let row: Option<<$db as sqlx::Database>::Row> = builder
                    .build()
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        cinderblock_core::ReadError::DataLayer(
                            format!("read from `{}`: {e}", R::TABLE_NAME).into(),
                        )
                    })?;

                match row {
                    Some(row) => R::from_row(&row).map_err(cinderblock_core::ReadError::DataLayer),
                    None => Err(cinderblock_core::ReadError::NotFound {
                        primary_key: primary_key.to_string(),
                    }),
                }
            }

            async fn update(&self, resource: R) -> Result<(), cinderblock_core::UpdateError> {
                let mut builder = sqlx::QueryBuilder::new(format!("UPDATE {} SET ", R::TABLE_NAME));

                resource.bind_update(&mut builder);

                builder.push(format!(" WHERE {} = ", R::PRIMARY_KEY_COLUMN));
                R::bind_primary_key(resource.primary_key(), &mut builder);

                builder.build().execute(self.pool()).await.map_err(|e| {
                    cinderblock_core::UpdateError::DataLayer(
                        format!("update `{}`: {e}", R::TABLE_NAME).into(),
                    )
                })?;

                Ok(())
            }

            async fn destroy(
                &self,
                primary_key: &R::PrimaryKey,
            ) -> Result<R, cinderblock_core::DestroyError> {
                let mut builder = sqlx::QueryBuilder::new(format!(
                    "DELETE FROM {} WHERE {} = ",
                    R::TABLE_NAME,
                    R::PRIMARY_KEY_COLUMN,
                ));

                R::bind_primary_key(primary_key, &mut builder);
                builder.push(" RETURNING *");

                let row: Option<<$db as sqlx::Database>::Row> = builder
                    .build()
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        cinderblock_core::DestroyError::DataLayer(
                            format!("destroy from `{}`: {e}", R::TABLE_NAME).into(),
                        )
                    })?;

                match row {
                    Some(row) => {
                        R::from_row(&row).map_err(cinderblock_core::DestroyError::DataLayer)
                    }
                    None => Err(cinderblock_core::DestroyError::NotFound {
                        primary_key: primary_key.to_string(),
                    }),
                }
            }
        }

        impl<R, A> cinderblock_core::PerformRead<A> for $dl
        where
            R: cinderblock_core::Resource + $crate::SqlResource<$db> + 'static,
            A: cinderblock_core::ReadAction<Output = R> + $crate::SqlPerformRead<$db> + 'static,
        {
            async fn read(
                &self,
                args: &A::Arguments,
            ) -> Result<A::Response, cinderblock_core::ListError> {
                A::execute(self.pool(), args).await
            }
        }

        impl<R, A> cinderblock_core::PerformReadOne<A> for $dl
        where
            R: cinderblock_core::Resource + $crate::SqlResource<$db> + 'static,
            A: cinderblock_core::ReadAction<Output = R> + $crate::SqlPerformReadOne<$db> + 'static,
        {
            async fn read_one(
                &self,
                args: &A::Arguments,
            ) -> Result<A::Response, cinderblock_core::ReadError> {
                A::execute(self.pool(), args).await
            }
        }
    };
}

pub(crate) use impl_sql_data_layer;
