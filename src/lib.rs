pub mod agent;
pub mod machinery;
pub(crate) mod utils;

pub fn greet(name: impl AsRef<str>) {
    println!("hello from {}", name.as_ref());
}
