//! Meeting AI abstraction layer for recording, transcription, and analysis providers.
//!
//! This crate provides trait-based abstractions for meeting AI workflows:
//! - Recording bots that join and record meetings
//! - Speech-to-text transcription with enhancements
//! - LLM-powered analysis and resource extraction
//!
//! The design is provider-agnostic, enabling applications to swap between
//! different service providers (Recall.ai, AssemblyAI, Deepgram, etc.) without
//! changing application code.

pub mod error;
pub mod traits;
pub mod types;

// Re-export commonly used types
pub use error::Error;
pub use types::analysis::ExtractedResource;
