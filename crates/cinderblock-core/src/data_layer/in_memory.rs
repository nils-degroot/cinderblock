use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use tokio::sync::RwLock;

use crate::{
    DestroyError, ListError, PerformRead, PerformReadOne, ReadAction, ReadError, Resource,
    UpdateError, data_layer::DataLayer,
};

type State =
    LazyLock<Arc<RwLock<HashMap<TypeId, HashMap<String, Box<dyn Any + Send + Sync + 'static>>>>>>;

static STATE: State = LazyLock::new(Arc::default);

#[derive(Debug)]
pub struct InMemoryDataLayer {}
impl InMemoryDataLayer {
    pub(crate) fn new() -> Self {
        Self {}
    }

    /// Load all rows of a given resource type from the global in-memory
    /// store.
    ///
    /// This is used by generated relation-loading code to batch-fetch
    /// related resources. Returns an empty `Vec` if no rows of the
    /// requested type exist.
    pub async fn load_all<R: Resource + 'static>(&self) -> Vec<R> {
        let state = STATE.clone();
        let state = state.read().await;

        state
            .get(&TypeId::of::<R>())
            .into_iter()
            .flat_map(|map| map.values())
            .filter_map(|boxed| boxed.downcast_ref::<R>())
            .cloned()
            .collect()
    }
}

impl<R: Resource + 'static> DataLayer<R> for InMemoryDataLayer {
    async fn create(&self, resource: R) -> Result<R, crate::CreateError> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let map = state.entry(TypeId::of::<R>()).or_default();
        map.insert(
            resource.primary_key().to_string(),
            Box::new(resource.clone()),
        );

        Ok(resource)
    }

    async fn read(&self, primary_key: &R::PrimaryKey) -> Result<R, crate::ReadError> {
        let state = STATE.clone();
        let state = state.read().await;

        let key = primary_key.to_string();

        state
            .get(&TypeId::of::<R>())
            .and_then(|map| map.get(&key))
            .and_then(|boxed| boxed.downcast_ref::<R>())
            .cloned()
            .ok_or_else(|| ReadError::NotFound { primary_key: key })
    }

    async fn update(&self, resource: R) -> Result<(), crate::UpdateError> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let key = resource.primary_key().to_string();

        let map = state
            .get_mut(&TypeId::of::<R>())
            .ok_or_else(|| UpdateError::NotFound {
                primary_key: key.clone(),
            })?;

        if !map.contains_key(&key) {
            return Err(UpdateError::NotFound { primary_key: key });
        }

        map.insert(key, Box::new(resource.clone()));

        Ok(())
    }

    async fn destroy(&self, primary_key: &R::PrimaryKey) -> Result<R, crate::DestroyError> {
        let state = STATE.clone();
        let mut state = state.write().await;

        let key = primary_key.to_string();

        let map = state
            .get_mut(&TypeId::of::<R>())
            .ok_or_else(|| DestroyError::NotFound {
                primary_key: key.clone(),
            })?;

        let boxed = map.remove(&key).ok_or_else(|| DestroyError::NotFound {
            primary_key: key.clone(),
        })?;

        boxed
            .downcast::<R>()
            .map(|r| *r)
            .map_err(|_| DestroyError::DataLayer("failed to downcast destroyed resource".into()))
    }
}

/// Filter trait for non-paged in-memory read actions. The generated
/// `resource!` macro emits an impl for each non-paged read action.
pub trait InMemoryReadAction: ReadAction {
    fn filter(row: &Self::Output, args: &Self::Arguments) -> bool;
}

/// Filter trait for paged in-memory read actions. Same filter interface as
/// [`InMemoryReadAction`], but applied to paged actions which additionally
/// require `Arguments: Paged`.
pub trait InMemoryPagedReadAction: ReadAction {
    fn filter(row: &Self::Output, args: &Self::Arguments) -> bool;
}

/// Unified execution trait that bridges the filter-based traits
/// ([`InMemoryReadAction`] and [`InMemoryPagedReadAction`]) to the
/// framework's [`PerformRead`] trait.
///
/// The `resource!` macro generates explicit impls of this trait for each
/// read action, delegating to the appropriate filter logic. A single
/// blanket `PerformRead` impl then dispatches to this trait.
pub trait InMemoryPerformRead: ReadAction {
    fn execute(all: impl Iterator<Item = Self::Output>, args: &Self::Arguments) -> Self::Response;
}

/// Single `PerformRead` impl for `InMemoryDataLayer` that delegates to
/// `InMemoryPerformRead::execute`.
impl<R, A> PerformRead<A> for InMemoryDataLayer
where
    R: Resource + 'static,
    A: ReadAction<Output = R> + InMemoryPerformRead + 'static,
{
    async fn read(&self, args: &A::Arguments) -> Result<A::Response, ListError> {
        let state = STATE.clone();
        let state = state.read().await;

        let type_map = state.get(&TypeId::of::<R>());
        let all = type_map
            .iter()
            .flat_map(|map| map.values())
            .filter_map(|boxed| boxed.downcast_ref::<R>())
            .cloned();

        Ok(A::execute(all, args))
    }
}

/// Single `PerformReadOne` impl for `InMemoryDataLayer` that delegates
/// to the underlying `DataLayer::read` by primary key.
///
/// Get-actions set `Arguments = PrimaryKey` and `Response = Resource`,
/// so this blanket impl covers all get-actions without per-action codegen.
impl<R, A> PerformReadOne<A> for InMemoryDataLayer
where
    R: Resource + 'static,
    A: ReadAction<Output = R, Arguments = R::PrimaryKey, Response = R> + 'static,
{
    async fn read_one(&self, args: &A::Arguments) -> Result<A::Response, ReadError> {
        <Self as DataLayer<R>>::read(self, args).await
    }
}
