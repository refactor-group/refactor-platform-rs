//! Types for transcription operations.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Processing status of a speech-to-text transcription job.
///
/// Jobs typically progress Queued → Processing → Completed within minutes.
/// Poll or use webhooks to monitor progress; avoid tight loops that waste API quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Processing,
    Completed,
    Failed,
}

/// Individual word with precise timing and speaker attribution.
///
/// Enables word-level highlighting, search, and navigation in transcript UIs.
/// Confidence scores help identify low-quality audio segments that may need review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f64,
    pub speaker: Option<String>,
}

/// Continuous speech segment (utterance) from a single speaker.
///
/// Represents natural speaking turns in conversation with speaker diarization.
/// Use segments for speaker attribution, conversation flow analysis, and UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    pub speaker: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f64,
    pub words: Vec<Word>,
}

/// Auto-detected topical chapter with AI-generated summary.
///
/// Providers use NLP to identify topic changes and create logical sections.
/// Useful for long meetings to help users navigate to relevant discussions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub summary: String,
    pub gist: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Emotional tone classification (positive, negative, neutral).
///
/// Use for conversation quality analysis, coaching feedback, or conflict detection.
/// Confidence below 0.7 suggests ambiguous emotional tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sentiment {
    Positive,
    Neutral,
    Negative,
}

/// Sentiment analysis for a segment of the transcript.
///
/// Links emotional tone to specific text, speaker, and timestamp for contextual analysis.
/// Aggregate sentiment scores provide meeting mood indicators and communication insights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentAnalysis {
    pub text: String,
    pub sentiment: Sentiment,
    pub confidence: f64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
}

/// Complete transcription with speech-to-text results and optional enhancements.
///
/// Fields populate based on enabled features in Config.
/// Poll get_transcription until status is Completed or Failed before accessing results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcription {
    pub id: String,
    pub status: Status,
    pub text: Option<String>,
    pub words: Vec<Word>,
    pub segments: Vec<Segment>,
    pub chapters: Vec<Chapter>,
    pub sentiment_analysis: Vec<SentimentAnalysis>,
    pub confidence: Option<f64>,
    pub duration_seconds: Option<i64>,
    pub language_code: Option<String>,
    pub speaker_count: Option<u32>,
    pub error_message: Option<String>,
}

/// Configuration for creating a transcription job.
///
/// Enable optional features (speaker labels, sentiment, chapters) via flags.
/// Set webhook_url to receive completion notification; otherwise poll get_transcription.
/// Provider_options allow vendor-specific tuning (e.g., custom vocabulary, punctuation).
#[derive(Debug, Clone)]
pub struct Config {
    pub media_url: String,
    pub webhook_url: Option<String>,
    pub enable_speaker_labels: bool,
    pub enable_sentiment_analysis: bool,
    pub enable_auto_chapters: bool,
    pub enable_entity_detection: bool,
    pub language_code: Option<String>,
    pub provider_options: HashMap<String, String>,
}
