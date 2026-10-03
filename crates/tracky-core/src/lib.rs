//! Tracky çekirdeği: platformdan bağımsız takip motoru, veri modeli ve yerel depolama.
//!
//! Platforma özel kod (aktif pencere / idle tespiti) [`platform::ActivityProvider`]
//! trait'ini uygular; motor ve depolama bu crate'te kalır ki testlerle doğrulanabilsin.

pub mod browser;
pub mod classify;
pub mod coach;
pub mod engine;
pub mod export;
pub mod focus;
pub mod model;
pub mod platform;
pub mod privacy;
pub mod report;
#[cfg(feature = "store")]
pub mod store;
#[cfg(feature = "store")]
pub mod sync;
#[cfg(feature = "store")]
pub mod tracker;
pub mod url_util;

pub use classify::{Classifier, Rule, RuleField, Tag, TagKind};
pub use coach::{Coach, Goals, Nudge};
pub use engine::{Engine, EngineConfig};
pub use model::{ActiveWindow, Session};
pub use platform::ActivityProvider;
pub use privacy::PrivacySettings;
pub use report::Report;
#[cfg(feature = "store")]
pub use store::{Store, StoreError, UsageTotal};
#[cfg(feature = "store")]
pub use tracker::{TickOutcome, Tracker};
