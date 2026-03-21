use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use tokio::sync::RwLock;

use crate::data_layer::DataLayer;

type State =
    LazyLock<Arc<RwLock<HashMap<TypeId, HashMap<String, Box<dyn Any + Send + Sync + 'static>>>>>>;

static STATE: State = LazyLock::new(Arc::default);

#[derive(Debug)]
pub struct InMemoryDataLayer {}
impl InMemoryDataLayer {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl DataLayer for InMemoryDataLayer {
    async fn create<R: crate::Resource + 'static>(&self, resource: R) -> crate::Result<()> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let map = state.entry(resource.type_id()).or_default();
        map.insert(
            resource.primary_key().to_string(),
            Box::new(resource.clone()),
        );

        Ok(())
    }

    async fn read<R: crate::Resource + 'static>(
        &self,
        primary_key: &R::PrimaryKey,
    ) -> crate::Result<R> {
        let state = STATE.clone();
        let state = state.read().await;

        let key = primary_key.to_string();

        state
            .get(&TypeId::of::<R>())
            .and_then(|map| map.get(&key))
            .and_then(|boxed| boxed.downcast_ref::<R>())
            .map(R::clone)
            .ok_or_else(|| format!("resource not found for primary key `{key}`").into())
    }

    async fn update<R: crate::Resource + 'static>(&self, resource: R) -> crate::Result<()> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let key = resource.primary_key().to_string();

        let map = state
            .get_mut(&resource.type_id())
            .ok_or_else(|| format!("resource not found for primary key `{key}`"))?;

        if !map.contains_key(&key) {
            return Err(format!("resource not found for primary key `{key}`").into());
        }

        map.insert(key, Box::new(resource.clone()));

        Ok(())
    }

    async fn list<R: crate::Resource + 'static>(&self) -> crate::Result<Vec<R>> {
        let state = STATE.clone();
        let state = state.read().await;

        Ok(state
            .get(&TypeId::of::<R>())
            .map(|map| {
                map.values()
                    .filter_map(|r| r.downcast_ref())
                    .map(R::clone)
                    .collect()
            })
            .unwrap_or_default())
    }
}
