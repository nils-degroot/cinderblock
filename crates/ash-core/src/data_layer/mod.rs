use crate::Resource;

pub mod file_storage;

pub trait DataLayer: std::fmt::Debug + Send + Sync + 'static {
    fn get<R: Resource>(
        &self,
        pk: R::PrimaryKey,
    ) -> impl Future<Output = crate::Result<Option<R>>> + Send;

    fn create<R: Resource>(&self, resource: &R) -> impl Future<Output = crate::Result<()>> + Send;
}
