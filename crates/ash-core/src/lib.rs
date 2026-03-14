pub use ash_core_macros::resource;

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

#[derive(Debug)]
pub struct Context {}

impl Context {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

pub trait Resource {}

pub trait Create<A>: Resource {
    type Input;

    fn from_create_input(input: Self::Input) -> Self;
}

pub fn create<R, A>(input: R::Input, _ctx: &Context) -> Result<R>
where
    R: Create<A>,
{
    let resource = R::from_create_input(input);
    Ok(resource)
}
