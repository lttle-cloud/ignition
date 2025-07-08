pub mod agent;
pub mod api;
pub mod machinery;
pub mod repository_test;
pub mod resources;
pub mod utils;

meta::include_build_mod!("repository");
meta::include_build_mod!("services");

pub fn greet(name: impl AsRef<str>) {
    println!("hello from {}", name.as_ref());
}
