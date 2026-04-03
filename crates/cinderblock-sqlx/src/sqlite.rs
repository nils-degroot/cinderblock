use sqlx::SqlitePool;

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

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

crate::impl_sql_data_layer!(SqliteDataLayer, sqlx::Sqlite);
