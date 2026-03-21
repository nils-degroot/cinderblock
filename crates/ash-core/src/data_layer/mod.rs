use crate::Resource;

pub mod in_memory;

pub trait DataLayer: std::fmt::Debug + Send + Sync + 'static {
    fn create<R: Resource + 'static>(
        &self,
        resource: R,
    ) -> impl Future<Output = crate::Result<()>> + Send;

    fn read<R: Resource + 'static>(
        &self,
        primary_key: &R::PrimaryKey,
    ) -> impl Future<Output = crate::Result<R>> + Send;

    fn update<R: Resource + 'static>(
        &self,
        resource: R,
    ) -> impl Future<Output = crate::Result<()>> + Send;

    fn list<R: Resource + 'static>(&self) -> impl Future<Output = crate::Result<Vec<R>>> + Send;

    /// Remove a resource by primary key, returning the deleted resource.
    fn destroy<R: Resource + 'static>(
        &self,
        primary_key: &R::PrimaryKey,
    ) -> impl Future<Output = crate::Result<R>> + Send;
}
