// # SQLite Data Layer
//
// Implements `DataLayer<R>` for any resource that also implements `SqlResource`.
// Queries are built dynamically using `sqlx::QueryBuilder` and the column
// metadata / bind helpers provided by the generated `SqlResource` impl.

use cinderblock_core::{
    CreateError, DestroyError, ListError, PerformRead, PerformReadOne, ReadAction, ReadError,
    Resource, UpdateError, data_layer::DataLayer,
};
use sqlx::{SqlitePool, sqlite::SqliteRow};

use crate::{SqlPerformRead, SqlPerformReadOne, SqlResource};

// ---------------------------------------------------------------------------
// # SqliteDataLayer
// ---------------------------------------------------------------------------

/// Persistence backend that stores resources in a SQLite database.
///
/// Each resource attribute maps to its own column. The table must already
/// exist with the correct schema — this layer does not run migrations.
///
/// # Construction
///
/// ```rust,ignore
/// let dl = SqliteDataLayer::new("sqlite::memory:").await?;
/// // or
/// let dl = SqliteDataLayer::new("sqlite:path/to/db.sqlite").await?;
/// ```
#[derive(Debug, Clone)]
pub struct SqliteDataLayer {
    pool: SqlitePool,
}

impl SqliteDataLayer {
    /// Connect to a SQLite database and return a new data layer.
    ///
    /// The `url` follows sqlx's connection string format:
    /// - `sqlite::memory:` for an in-memory database
    /// - `sqlite:path/to/file.db` for a file-backed database
    pub async fn new(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let pool = SqlitePool::connect(url)
            .await
            .map_err(|e| format!("connect to SQLite database: {e}"))?;

        Ok(Self { pool })
    }

    /// Access the underlying connection pool.
    ///
    /// Useful for running raw SQL (e.g., schema setup in tests) outside
    /// of the `DataLayer` trait methods.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

// ---------------------------------------------------------------------------
// # DataLayer Implementation
// ---------------------------------------------------------------------------

impl<R> DataLayer<R> for SqliteDataLayer
where
    R: Resource + SqlResource + 'static,
{
    // # INSERT with RETURNING
    //
    // Builds: INSERT INTO {table} ({col1}, {col2}, ...) VALUES (?, ?, ...) RETURNING *
    //
    // Uses `INSERT_COLUMN_NAMES` instead of `COLUMN_NAMES` so that columns
    // marked `generated true` are omitted — their values come from the
    // database (e.g. autoincrement PKs, server-side DEFAULT expressions).
    //
    // The RETURNING * clause lets us get back the full row including
    // database-generated values in a single round-trip.
    async fn create(&self, resource: R) -> Result<R, CreateError> {
        let columns = R::INSERT_COLUMN_NAMES.join(", ");

        let mut builder = sqlx::QueryBuilder::new(format!(
            "INSERT INTO {} ({}) VALUES (",
            R::TABLE_NAME,
            columns,
        ));

        resource.bind_insert(&mut builder);
        builder.push(") RETURNING *");

        let row: SqliteRow = builder.build().fetch_one(&self.pool).await.map_err(|e| {
            CreateError::DataLayer(format!("insert into `{}`: {e}", R::TABLE_NAME).into())
        })?;

        R::from_row(&row).map_err(CreateError::DataLayer)
    }

    // # SELECT by primary key
    //
    // Builds: SELECT * FROM {table} WHERE {pk_col} = ?
    async fn read(&self, primary_key: &R::PrimaryKey) -> Result<R, ReadError> {
        let mut builder = sqlx::QueryBuilder::new(format!(
            "SELECT * FROM {} WHERE {} = ",
            R::TABLE_NAME,
            R::PRIMARY_KEY_COLUMN,
        ));

        R::bind_primary_key(primary_key, &mut builder);

        let row: Option<SqliteRow> =
            builder
                .build()
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    ReadError::DataLayer(format!("read from `{}`: {e}", R::TABLE_NAME).into())
                })?;

        match row {
            Some(row) => R::from_row(&row).map_err(ReadError::DataLayer),
            None => Err(ReadError::NotFound {
                primary_key: primary_key.to_string(),
            }),
        }
    }

    // # UPDATE
    //
    // Builds: UPDATE {table} SET col1 = ?, col2 = ? WHERE {pk_col} = ?
    //
    // `bind_update` emits the `col = ?` pairs for non-PK columns, then
    // we append the WHERE clause and bind the primary key separately.
    async fn update(&self, resource: R) -> Result<(), UpdateError> {
        let mut builder = sqlx::QueryBuilder::new(format!("UPDATE {} SET ", R::TABLE_NAME));

        resource.bind_update(&mut builder);

        builder.push(format!(" WHERE {} = ", R::PRIMARY_KEY_COLUMN));
        R::bind_primary_key(resource.primary_key(), &mut builder);

        builder.build().execute(&self.pool).await.map_err(|e| {
            UpdateError::DataLayer(format!("update `{}`: {e}", R::TABLE_NAME).into())
        })?;

        Ok(())
    }

    // # DELETE with RETURNING
    //
    // Builds: DELETE FROM {table} WHERE {pk_col} = ? RETURNING *
    //
    // SQLite supports RETURNING since version 3.35 (2021-03-12). This
    // lets us atomically delete and return the resource in a single query.
    async fn destroy(&self, primary_key: &R::PrimaryKey) -> Result<R, DestroyError> {
        let mut builder = sqlx::QueryBuilder::new(format!(
            "DELETE FROM {} WHERE {} = ",
            R::TABLE_NAME,
            R::PRIMARY_KEY_COLUMN,
        ));

        R::bind_primary_key(primary_key, &mut builder);
        builder.push(" RETURNING *");

        let row: Option<SqliteRow> =
            builder
                .build()
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    DestroyError::DataLayer(format!("destroy from `{}`: {e}", R::TABLE_NAME).into())
                })?;

        match row {
            Some(row) => R::from_row(&row).map_err(DestroyError::DataLayer),
            None => Err(DestroyError::NotFound {
                primary_key: primary_key.to_string(),
            }),
        }
    }
}

/// Single `PerformRead` impl that delegates to `SqlPerformRead::execute`.
///
/// The `cinderblock_sqlx` extension macro generates `SqlPerformRead` impls
/// for each read action, routing to `SqlReadAction::execute` (non-paged)
/// or `SqlPagedReadAction::execute` (paged) as appropriate.
impl<R, A> PerformRead<A> for SqliteDataLayer
where
    R: Resource + SqlResource + 'static,
    A: ReadAction<Output = R> + SqlPerformRead + 'static,
{
    async fn read(&self, args: &A::Arguments) -> Result<A::Response, ListError> {
        A::execute(&self.pool, args).await
    }
}

impl<R, A> PerformReadOne<A> for SqliteDataLayer
where
    R: Resource + SqlResource + 'static,
    A: ReadAction<Output = R> + SqlPerformReadOne + 'static,
{
    async fn read_one(&self, args: &A::Arguments) -> Result<A::Response, ReadError> {
        A::execute(&self.pool, args).await
    }
}
