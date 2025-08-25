#[cfg(feature = "daemon")]
pub mod agent;
#[cfg(feature = "daemon")]
pub mod api;
pub mod constants;
#[cfg(feature = "daemon")]
pub mod controller;
pub mod machinery;
pub mod resources;
pub mod utils;

#[cfg(feature = "daemon")]
meta::include_build_mod!("repository");
#[cfg(feature = "daemon")]
meta::include_build_mod!("services");
meta::include_build_mod!("resource_index");
meta::include_build_mod!("api_client");
