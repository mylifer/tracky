//! Tracky çekirdeği: platformdan bağımsız takip motoru, veri modeli ve yerel depolama.
//!
//! Platforma özel kod (aktif pencere / idle tespiti) [`platform::ActivityProvider`]
//! trait'ini uygular; motor ve depolama bu crate'te kalır ki testlerle doğrulanabilsin.

pub mod ai;
pub mod blocks;
pub mod browser;
pub mod budget;
pub mod calendar;
pub mod classify;
pub mod client_report;
pub mod coach;
pub mod engine;
pub mod export;
pub mod favicon;
pub mod inbox;
pub mod learn;
pub mod meeting_suggest;
pub mod model;
pub mod platform;
pub mod privacy;
pub mod project_stats;
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

pub use classify::{Classifier, Client, NO_PROJECT, Rule, RuleField, Tag, TagKind};
pub use coach::{CategoryLimit, Coach, Goals, Nudge, ProjectGoal, export_reminder_due};
pub use engine::{Engine, EngineConfig};
pub use model::{ActiveWindow, MANUAL_APP_ID, Session};
pub use platform::ActivityProvider;
pub use privacy::PrivacySettings;
pub use report::Report;
#[cfg(feature = "store")]
pub use store::{BackupInfo, SavedEntry, Store, StoreError, UsageTotal};
#[cfg(feature = "store")]
pub use tracker::{TickOutcome, Tracker};
