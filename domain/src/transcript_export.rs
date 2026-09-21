//! Speaker resolution and plain-text rendering for transcript downloads.

use crate::error::Error;
use crate::users;
use chrono::NaiveDate;
use entity::transcript_segment::Model as Segment;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Which participant of the coaching relationship a speaker label resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerRole {
    Coach,
    Coachee,
}

/// A distinct transcript speaker label and the participant it resolved to, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct Speaker {
    pub label: String,
    pub role: Option<SpeakerRole>,
}

/// A rendered plain-text transcript and the filename to serve it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub body: String,
    pub filename: String,
}

pub fn resolve_speakers(
    _coach: &users::Model,
    _coachee: &users::Model,
    _segments: &[Segment],
) -> Vec<Speaker> {
    todo!("Phase 2")
}

pub fn render_plain_text(
    _session_date: NaiveDate,
    _speakers: &[Speaker],
    _segments: &[Segment],
    _filter: &[SpeakerRole],
) -> Result<Rendered, Error> {
    todo!("Phase 2")
}

// Scaffold only: the allow goes away once Phase 2 calls this from the renderer.
#[allow(dead_code)]
fn format_timestamp(_ms: i32) -> String {
    todo!("Phase 2")
}

#[cfg(test)]
#[path = "transcript_export_tests.rs"]
mod tests;
