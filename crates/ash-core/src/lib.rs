use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

pub use ash_core_macros::resource;
pub use serde;

use crate::data_layer::DataLayer;

pub mod data_layer;

pub type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = std::result::Result<T, E>;

#[derive(Debug)]
pub struct Context {
    data_layers: HashMap<TypeId, Box<dyn Any + Sync + Send + 'static>>,
}

impl Context {
    pub async fn new(_app_name: &str) -> Result<Self> {
        let mut this = Self {
            data_layers: HashMap::new(),
        };

        this.register_data_layer(data_layer::in_memory::InMemoryDataLayer::new());

        Ok(this)
    }

    pub fn register_data_layer<DL: DataLayer>(&mut self, data_layer: DL) {
        self.data_layers
            .insert(data_layer.type_id(), Box::new(data_layer));
    }

    pub fn get_data_layer<DL: DataLayer>(&self) -> &DL {
        self.data_layers
            .get(&TypeId::of::<DL>())
            .expect("Requested data layer was not registered")
            .downcast_ref()
            .expect("Could not downcast value stored daya layer")
    }
}

pub trait Resource: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + Clone {
    type PrimaryKey: std::fmt::Display + serde::de::DeserializeOwned + Send + Sync;

    type DataLayer: DataLayer;

    const NAME: &'static [&'static str];

    const PRIMARY_KEY_GENERATED: bool;

    fn primary_key(&self) -> &Self::PrimaryKey;
}

pub trait Create<A>: Resource {
    type Input;

    fn from_create_input(input: Self::Input) -> Self;
}

pub trait Update<A>: Resource {
    type Input;

    fn apply_update_input(&mut self, input: Self::Input);
}

/// Marker trait for destroy actions. Unlike create and update, destroy
/// actions take no input — the primary key is sufficient to identify the
/// resource to delete.
pub trait Destroy<A>: Resource {}

pub async fn create<R, A>(input: R::Input, ctx: &Context) -> Result<R>
where
    R: Create<A> + 'static,
{
    let resource = R::from_create_input(input);
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.create(resource.clone()).await?;
    Ok(resource)
}

pub async fn update<R, A>(primary_key: &R::PrimaryKey, input: R::Input, ctx: &Context) -> Result<R>
where
    R: Update<A> + 'static,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    let mut resource = dl.read::<R>(primary_key).await?;
    resource.apply_update_input(input);
    dl.update(resource.clone()).await?;
    Ok(resource)
}

pub async fn list<R>(ctx: &Context) -> Result<Vec<R>>
where
    R: Resource + 'static,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.list().await
}

pub async fn destroy<R, A>(primary_key: &R::PrimaryKey, ctx: &Context) -> Result<R>
where
    R: Destroy<A> + 'static,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.destroy(primary_key).await
}
