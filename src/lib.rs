pub mod config;
pub mod connectors;
pub mod daemon;
pub mod db;
pub mod distill;
pub mod embed;
pub mod hooks;
pub mod llm;
pub mod mcp;
pub mod models;
pub mod project;
pub mod scrub;
pub mod server;
pub mod t3_watcher;
pub mod updater;

pub use db::register_sqlite_vec;
