//! Tracky çekirdeği: platformdan bağımsız takip motoru, veri modeli ve yerel depolama.
//!
//! Platforma özel kod (aktif pencere / idle tespiti) [`platform::ActivityProvider`]
//! trait'ini uygular; motor ve depolama bu crate'te kalır ki testlerle doğrulanabilsin.

pub mod engine;
pub mod model;
pub mod platform;
pub mod store;
pub mod url_util;

pub use engine::{Engine, EngineConfig};
pub use model::{ActiveWindow, Session};
pub use platform::ActivityProvider;
pub use store::{Store, StoreError, UsageTotal};
