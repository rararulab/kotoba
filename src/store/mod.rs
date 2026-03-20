//! SQLite store layer — adapted from yunara-store.

mod config;
mod db;
mod err;

pub use config::DatabaseConfig;
pub use db::DBStore;
pub use err::Error as StoreError;
