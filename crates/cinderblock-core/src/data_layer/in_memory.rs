use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use tokio::sync::RwLock;

use crate::{PerformRead, ReadAction, Resource, data_layer::DataLayer};

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

impl<R: Resource + 'static> DataLayer<R> for InMemoryDataLayer {
    async fn create(&self, resource: R) -> crate::Result<R> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let map = state.entry(TypeId::of::<R>()).or_default();
        map.insert(
            resource.primary_key().to_string(),
            Box::new(resource.clone()),
        );

        Ok(resource)
    }

    async fn read(&self, primary_key: &R::PrimaryKey) -> crate::Result<R> {
        let state = STATE.clone();
        let state = state.read().await;

        let key = primary_key.to_string();

        state
            .get(&TypeId::of::<R>())
            .and_then(|map| map.get(&key))
            .and_then(|boxed| boxed.downcast_ref::<R>())
            .cloned()
            .ok_or_else(|| format!("resource not found for primary key `{key}`").into())
    }

    async fn update(&self, resource: R) -> crate::Result<()> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let key = resource.primary_key().to_string();

        let map = state
            .get_mut(&TypeId::of::<R>())
            .ok_or_else(|| format!("resource not found for primary key `{key}`"))?;

        if !map.contains_key(&key) {
            return Err(format!("resource not found for primary key `{key}`").into());
        }

        map.insert(key, Box::new(resource.clone()));

        Ok(())
    }

    async fn destroy(&self, primary_key: &R::PrimaryKey) -> crate::Result<R> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let key = primary_key.to_string();

        let map = state
            .get_mut(&TypeId::of::<R>())
            .ok_or_else(|| format!("resource not found for primary key `{key}`"))?;

        let boxed = map
            .remove(&key)
            .ok_or_else(|| format!("resource not found for primary key `{key}`"))?;

        boxed
            .downcast::<R>()
            .map(|r| *r)
            .map_err(|_| "failed to downcast destroyed resource".into())
    }
}

pub trait InMemoryReadAction: ReadAction {
    fn filter(row: &Self::Output, args: &Self::Arguments) -> bool;
}

impl<R, A> PerformRead<A> for InMemoryDataLayer
where
    R: Resource + 'static,
    A: ReadAction<Output = R> + InMemoryReadAction + 'static,
{
    async fn read(&self, args: &A::Arguments) -> crate::Result<Vec<A::Output>> {
        let state = STATE.clone();
        let state = state.read().await;

        Ok(state
            .get(&TypeId::of::<R>())
            .iter()
            .flat_map(|map| map.values())
            .filter_map(|boxed| boxed.downcast_ref::<R>())
            .filter(|row| A::filter(row, args))
            .cloned()
            .collect())
    }
}
