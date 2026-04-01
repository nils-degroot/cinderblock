use crate::Resource;

pub mod in_memory;

/// Persistence backend for a specific resource type.
///
/// The trait is parameterized on `R` so that different data layers can
/// impose different bounds on the resources they support. For example,
/// `InMemoryDataLayer` has a blanket impl for all `R: Resource`, while
/// a SQLite data layer can additionally require `R: SqlResource`.
pub trait DataLayer<R: Resource>: std::fmt::Debug + Send + Sync + 'static {
    /// Persist a new resource and return the stored version.
    ///
    /// The returned resource may differ from the input when the data layer
    /// provides server-generated values (e.g. autoincrement primary keys).
    fn create(&self, resource: R) -> impl Future<Output = crate::Result<R>> + Send;

    fn read(&self, primary_key: &R::PrimaryKey) -> impl Future<Output = crate::Result<R>> + Send;

    fn update(&self, resource: R) -> impl Future<Output = crate::Result<()>> + Send;

    /// Remove a resource by primary key, returning the deleted resource.
    fn destroy(&self, primary_key: &R::PrimaryKey)
    -> impl Future<Output = crate::Result<R>> + Send;
}
