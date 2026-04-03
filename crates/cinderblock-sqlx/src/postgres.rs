use sqlx::PgPool;

/// Persistence backend that stores resources in a PostgreSQL database.
///
/// Each resource attribute maps to its own column. The table must already
/// exist with the correct schema — this layer does not run migrations.
///
/// # Construction
///
/// ```rust,ignore
/// let dl = PostgresDataLayer::new("postgres://user:pass@localhost/mydb").await?;
/// ```
#[derive(Debug, Clone)]
pub struct PostgresDataLayer {
    pool: PgPool,
}

impl PostgresDataLayer {
    pub async fn new(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let pool = PgPool::connect(url)
            .await
            .map_err(|e| format!("connect to PostgreSQL database: {e}"))?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

crate::impl_sql_data_layer!(PostgresDataLayer, sqlx::Postgres);
