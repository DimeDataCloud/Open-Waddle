//! Project Waddle core: the Brain (planner loop, quick-reply lane, providers),
//! the Hands that don't need a GUI (files, commands), and the safety layer
//! (permission tiers, untrusted-content wrapping, hash-chained audit log).
//!
//! Everything OS-specific (overlay, mouse, keyboard, screenshots, accessibility)
//! lives in the Tauri app and reaches the core through [`agent::Host`].

pub mod agent;
pub mod audit;
pub mod config;
pub mod diagnostics;
pub mod llm;
pub mod safety;
pub mod session;
pub mod skills;
pub mod stt;
pub mod tools;
pub mod decide;
pub mod reminders;
pub mod traces;
pub mod untrusted;

pub use agent::{AgentEvent, ApprovalRequest, Decision, EnvInfo, Host, Lane, Outcome};
pub use config::Settings;
pub use session::{Session, SessionConfig};
