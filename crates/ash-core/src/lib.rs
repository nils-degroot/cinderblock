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
    type PrimaryKey: std::fmt::Display + Send + Sync;

    type DataLayer: DataLayer;

    const NAME: &'static [&'static str];

    const PRIMARY_KEY_GENERATED: bool;

    fn primary_key(&self) -> &Self::PrimaryKey;
}

pub trait Create<A>: Resource {
    type Input;

    fn from_create_input(input: Self::Input) -> Self;
}

pub async fn create<R, A>(input: R::Input, ctx: &Context) -> Result<()>
where
    R: Create<A> + 'static,
{
    let resource = R::from_create_input(input);
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.create(resource).await?;
    Ok(())
}

pub async fn list<R>(ctx: &Context) -> Result<Vec<R>>
where
    R: Resource + 'static,
{
    let dl = ctx.get_data_layer::<R::DataLayer>();
    dl.list().await
}
