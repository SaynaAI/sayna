pub mod config;
pub mod core;
pub mod errors;
pub mod handlers;
pub mod routes;
pub mod state;
pub mod utils;

// Re-export commonly used items for convenience
pub use config::ServerConfig;
pub use core::*;
pub use errors::app_error::{AppError, AppResult};
pub use state::AppState;
