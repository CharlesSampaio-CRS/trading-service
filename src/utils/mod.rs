// Utility functions
pub mod cache;
pub mod error;
pub mod crypto;
pub mod thread_pool;  // 🚀 FASE 3: Thread pool dedicado

pub use cache::*;
pub use error::*;
pub use crypto::*;
pub use thread_pool::*;
