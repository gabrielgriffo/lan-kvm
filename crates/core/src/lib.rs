pub mod audio;
pub mod ballistics;
pub mod clipboard;
pub mod config;
pub mod ddc;
pub mod frame;
pub mod keymap;
pub mod keys;
pub mod pairing;
pub mod proto;
pub mod puppet;
pub mod secure;
pub mod switch;
pub mod tap;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
