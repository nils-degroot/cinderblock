// # SQLite Data Layer
//
// Implements `DataLayer<R>` for any resource that also implements `SqlResource`.
// Queries are built dynamically using `sqlx::QueryBuilder` and the column
// metadata / bind helpers provided by the generated `SqlResource` impl.

use ash_core::data_layer::DataLayer;
use sqlx::{sqlite::SqliteRow, SqlitePool};

use crate::SqlResource;

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
    pub async fn new(url: &str) -> ash_core::Result<Self> {
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
    R: ash_core::Resource + SqlResource + 'static,
{
    // # INSERT
    //
    // Builds: INSERT INTO {table} ({col1}, {col2}, ...) VALUES (?, ?, ...)
    // and binds all column values via `SqlResource::bind_insert`.
    async fn create(&self, resource: R) -> ash_core::Result<()> {
        let columns = R::COLUMN_NAMES.join(", ");

        let mut builder = sqlx::QueryBuilder::new(format!(
            "INSERT INTO {} ({}) VALUES (",
            R::TABLE_NAME,
            columns,
        ));

        resource.bind_insert(&mut builder);
        builder.push(")");

        builder
            .build()
            .execute(&self.pool)
            .await
            .map_err(|e| format!("insert into `{}`: {e}", R::TABLE_NAME))?;

        Ok(())
    }

    // # SELECT by primary key
    //
    // Builds: SELECT * FROM {table} WHERE {pk_col} = ?
    async fn read(&self, primary_key: &R::PrimaryKey) -> ash_core::Result<R> {
        let mut builder = sqlx::QueryBuilder::new(format!(
            "SELECT * FROM {} WHERE {} = ",
            R::TABLE_NAME,
            R::PRIMARY_KEY_COLUMN,
        ));

        R::bind_primary_key(primary_key, &mut builder);

        let row: SqliteRow = builder
            .build()
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("read from `{}`: {e}", R::TABLE_NAME))?;

        R::from_row(&row)
    }

    // # UPDATE
    //
    // Builds: UPDATE {table} SET col1 = ?, col2 = ? WHERE {pk_col} = ?
    //
    // `bind_update` emits the `col = ?` pairs for non-PK columns, then
    // we append the WHERE clause and bind the primary key separately.
    async fn update(&self, resource: R) -> ash_core::Result<()> {
        let mut builder =
            sqlx::QueryBuilder::new(format!("UPDATE {} SET ", R::TABLE_NAME));

        resource.bind_update(&mut builder);

        builder.push(format!(" WHERE {} = ", R::PRIMARY_KEY_COLUMN));
        R::bind_primary_key(resource.primary_key(), &mut builder);

        builder
            .build()
            .execute(&self.pool)
            .await
            .map_err(|e| format!("update `{}`: {e}", R::TABLE_NAME))?;

        Ok(())
    }

    // # SELECT all
    //
    // Builds: SELECT * FROM {table}
    async fn list(&self) -> ash_core::Result<Vec<R>> {
        let sql = format!("SELECT * FROM {}", R::TABLE_NAME);

        let rows: Vec<SqliteRow> = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| format!("list from `{}`: {e}", R::TABLE_NAME))?;

        rows.iter().map(R::from_row).collect()
    }

    // # DELETE with RETURNING
    //
    // Builds: DELETE FROM {table} WHERE {pk_col} = ? RETURNING *
    //
    // SQLite supports RETURNING since version 3.35 (2021-03-12). This
    // lets us atomically delete and return the resource in a single query.
    async fn destroy(&self, primary_key: &R::PrimaryKey) -> ash_core::Result<R> {
        let mut builder = sqlx::QueryBuilder::new(format!(
            "DELETE FROM {} WHERE {} = ",
            R::TABLE_NAME,
            R::PRIMARY_KEY_COLUMN,
        ));

        R::bind_primary_key(primary_key, &mut builder);
        builder.push(" RETURNING *");

        let row: SqliteRow = builder
            .build()
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("destroy from `{}`: {e}", R::TABLE_NAME))?;

        R::from_row(&row)
    }
}
