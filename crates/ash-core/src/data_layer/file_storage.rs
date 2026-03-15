use std::path::PathBuf;

use crate::data_layer::DataLayer;

#[derive(Debug)]
pub struct FileStorageDataLayer {
    base_path: PathBuf,
}
impl FileStorageDataLayer {
    pub(crate) async fn new(base_path: PathBuf) -> crate::Result<Self> {
        tokio::fs::create_dir_all(&base_path).await?;

        Ok(Self { base_path })
    }
}

impl DataLayer for FileStorageDataLayer {
    async fn get<R: crate::Resource>(&self, _pk: R::PrimaryKey) -> crate::Result<Option<R>> {
        todo!()
    }

    async fn create<R: crate::Resource>(&self, resource: &R) -> crate::Result<()> {
        let resource_path = R::NAME
            .iter()
            .fold(self.base_path.clone(), |acc, v| acc.join(v));

        tokio::fs::create_dir_all(&resource_path).await?;

        let data = serde_json::to_vec(resource)?;

        tokio::fs::write(
            resource_path.join(format!("{}.json", resource.primary_key())),
            data,
        )
        .await?;

        Ok(())
    }
}
