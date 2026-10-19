use std::collections::HashMap;

pub use stormchaser_model::api::{
    HydrateDirectRequest, HydrateSchemaRequest, HydrationEvent, HydrationStatus,
};

#[derive(Clone, Debug)]
pub struct QueryTask {
    pub field_name: String,
    pub query_type: String,
    pub params: HashMap<String, String>,
    pub dependencies: Vec<String>,
    pub status: String,
}
