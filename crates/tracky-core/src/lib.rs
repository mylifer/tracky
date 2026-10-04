//! Tracky çekirdeği: platformdan bağımsız takip motoru, veri modeli ve yerel depolama.
//!
//! Platforma özel kod (aktif pencere / idle tespiti) [`platform::ActivityProvider`]
//! trait'ini uygular; motor ve depolama bu crate'te kalır ki testlerle doğrulanabilsin.

pub mod browser;
pub mod calendar;
pub mod classify;
pub mod coach;
pub mod engine;
pub mod export;
pub mod focus;
pub mod model;
pub mod platform;
pub mod privacy;
pub mod report;
pub mod search;
#[cfg(feature = "store")]
pub mod store;
pub mod suggest;
#[cfg(feature = "store")]
pub mod sync;
pub mod timesheet;
#[cfg(feature = "store")]
pub mod tracker;
pub mod trends;
pub mod url_util;

pub use classify::{Classifier, NO_PROJECT, Rule, RuleField, Tag, TagKind};
pub use coach::{CategoryLimit, Coach, Goals, Nudge, ProjectGoal};
pub use engine::{Engine, EngineConfig};
pub use model::{ActiveWindow, FocusTimer, MANUAL_APP_ID, Session};
pub use platform::ActivityProvider;
pub use privacy::PrivacySettings;
pub use report::Report;
#[cfg(feature = "store")]
pub use store::{SavedEntry, Store, StoreError, UsageTotal};
#[cfg(feature = "store")]
pub use tracker::{TickOutcome, Tracker};
