use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

pub use cinderblock_core_macros::resource;
pub use serde;

use crate::data_layer::DataLayer;

pub mod data_layer;

pub type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = std::result::Result<T, E>;

#[derive(Debug, Default)]
pub struct Context {
    data_layers: HashMap<TypeId, Box<dyn Any + Sync + Send + 'static>>,
}

impl Context {
    /// Generate a new context to be used by cinderblock applications.
    ///
    /// # Data layers
    ///
    /// This methods adds a [`data_layer::in_memory::InMemoryDataLayer`] by default.
    pub fn new() -> Self {
        let mut this = Self::default();
        this.register_data_layer(data_layer::in_memory::InMemoryDataLayer::new());
        this
    }

    /// Register a data layer instance so resources can look it up at runtime.
    pub fn register_data_layer<DL: std::fmt::Debug + Send + Sync + 'static>(
        &mut self,
        data_layer: DL,
    ) {
        self.data_layers
            .insert(data_layer.type_id(), Box::new(data_layer));
    }

    fn get_data_layer<DL: 'static>(&self) -> &DL {
        self.data_layers
            .get(&TypeId::of::<DL>())
            .expect("Requested data layer was not registered")
            .downcast_ref()
            .expect("Could not downcast value stored in data layer")
    }
}

/// Marker trait for a resource.
pub trait Resource:
    serde::Serialize + serde::de::DeserializeOwned + Send + Sync + Clone + 'static
{
    /// Primary key type of the resource. Usually the type of the id for the resource.
    type PrimaryKey: std::fmt::Display + serde::de::DeserializeOwned + Send + Sync;

    /// Data layer that the resource uses.
    type DataLayer: DataLayer<Self>;

    /// Name with namespace of the resource. Each part of the array is a segment in the name
    /// (i.e. MyApp.Blog.Post).
    const NAME: &'static [&'static str];

    /// Wether the primary key of the resource is generated
    const PRIMARY_KEY_GENERATED: bool;

    /// Mathos that returns the primary key of the resource
    fn primary_key(&self) -> &Self::PrimaryKey;
}

/// Marker trait showing indicating that a struct is a read action.
pub trait ReadAction {
    /// Resource returned when calling the action.
    type Output: Resource;

    /// Arguments used to get the resource. Could be used in filters.
    type Arguments: Sync;
}

/// Trait indicating that a [`DataLayer`] can perform [`ReadAction`] [`A`].
pub trait PerformRead<A: ReadAction> {
    /// Perform the read action on the provided data layer.
    fn read(&self, args: &A::Arguments) -> impl Future<Output = Result<Vec<A::Output>>>;
}

/// Trait placed on a [`Resource`] specifying how to create the resource using action [`A`].
pub trait Create<A>: Resource {
    /// Input used to create the resource.
    type Input;

    /// Create an instance of the resource using [`Self::Input`].
    fn from_create_input(input: Self::Input) -> Self;
}

/// Trait placed on a [`Resource`] specifying how to update a resource using action [`A`].
pub trait Update<A>: Resource {
    /// Arguments to pass to [`Self::apply_update_input`].
    type Input;

    /// Update an instance of self using [`Self::Input`].
    fn apply_update_input(&mut self, input: Self::Input);
}

/// Marker trait for destroy actions.
pub trait Destroy<A>: Resource {}

/// Create resource [`R`] using action [`A`].
pub async fn create<R, A>(input: R::Input, ctx: &Context) -> Result<R>
where
    R: Create<A>,
{
    let resource = R::from_create_input(input);
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.create(resource.clone()).await?;
    Ok(resource)
}

/// Update resource [`R`] using action [`A`]. First
/// fetched a instance of [`R`] using the PK of the resource.
pub async fn update<R, A>(primary_key: &R::PrimaryKey, input: R::Input, ctx: &Context) -> Result<R>
where
    R: Update<A>,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    let mut resource = dl.read(primary_key).await?;
    resource.apply_update_input(input);
    dl.update(resource.clone()).await?;
    Ok(resource)
}

/// Read resource [`R`] using action [`A`].
pub async fn read<R, A>(ctx: &Context, args: &A::Arguments) -> Result<Vec<R>>
where
    R: Resource,
    A: ReadAction<Output = R>,
    R::DataLayer: PerformRead<A>,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    PerformRead::<A>::read(dl, args).await
}

/// Destroy resource [`R`] using action [`A`].
pub async fn destroy<R, A>(primary_key: &R::PrimaryKey, ctx: &Context) -> Result<R>
where
    R: Destroy<A>,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.destroy(primary_key).await
}
