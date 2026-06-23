use serde::Serialize;
use serde_json::Value;

/// Trait for getting the SSE event type name
pub trait EventType {
    fn event_type(&self) -> &'static str;
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    // Actions (session-scoped)
    #[serde(rename = "action_created")]
    ActionCreated {
        coaching_session_id: String,
        action: Value,
    },
    #[serde(rename = "action_updated")]
    ActionUpdated {
        coaching_session_id: String,
        action: Value,
    },
    #[serde(rename = "action_deleted")]
    ActionDeleted {
        coaching_session_id: String,
        action_id: String,
    },

    // Agreements (relationship-scoped)
    #[serde(rename = "agreement_created")]
    AgreementCreated {
        coaching_relationship_id: String,
        agreement: Value,
    },
    #[serde(rename = "agreement_updated")]
    AgreementUpdated {
        coaching_relationship_id: String,
        agreement: Value,
    },
    #[serde(rename = "agreement_deleted")]
    AgreementDeleted {
        coaching_relationship_id: String,
        agreement_id: String,
    },

    // Goals (relationship-scoped)
    #[serde(rename = "goal_created")]
    GoalCreated {
        coaching_relationship_id: String,
        goal: Value,
    },
    #[serde(rename = "goal_updated")]
    GoalUpdated {
        coaching_relationship_id: String,
        goal: Value,
    },
    #[serde(rename = "goal_deleted")]
    GoalDeleted {
        coaching_relationship_id: String,
        goal_id: String,
    },

    // Coaching Session Goals (join table, relationship-scoped)
    #[serde(rename = "coaching_session_goal_created")]
    CoachingSessionGoalCreated {
        coaching_relationship_id: String,
        coaching_session_id: String,
        goal_id: String,
    },
    #[serde(rename = "coaching_session_goal_deleted")]
    CoachingSessionGoalDeleted {
        coaching_relationship_id: String,
        coaching_session_id: String,
        goal_id: String,
    },

    // System events
    #[serde(rename = "force_logout")]
    ForceLogout { reason: String },

    // Meeting recording events (session-scoped)
    #[serde(rename = "meeting_recording_updated")]
    MeetingRecordingUpdated { coaching_session_id: String },

    // Topic events (session-scoped, coarse: refetch on receipt)
    #[serde(rename = "topics_changed")]
    TopicsChanged { coaching_session_id: String },

    // Transcription events (session-scoped)
    #[serde(rename = "transcription_updated")]
    TranscriptionUpdated { coaching_session_id: String },
}

impl EventType for Event {
    fn event_type(&self) -> &'static str {
        match self {
            Event::ActionCreated { .. } => "action_created",
            Event::ActionUpdated { .. } => "action_updated",
            Event::ActionDeleted { .. } => "action_deleted",
            Event::AgreementCreated { .. } => "agreement_created",
            Event::AgreementUpdated { .. } => "agreement_updated",
            Event::AgreementDeleted { .. } => "agreement_deleted",
            Event::GoalCreated { .. } => "goal_created",
            Event::GoalUpdated { .. } => "goal_updated",
            Event::GoalDeleted { .. } => "goal_deleted",
            Event::CoachingSessionGoalCreated { .. } => "coaching_session_goal_created",
            Event::CoachingSessionGoalDeleted { .. } => "coaching_session_goal_deleted",
            Event::ForceLogout { .. } => "force_logout",
            Event::MeetingRecordingUpdated { .. } => "meeting_recording_updated",
            Event::TopicsChanged { .. } => "topics_changed",
            Event::TranscriptionUpdated { .. } => "transcription_updated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub event: Event,
    pub scope: MessageScope,
}

#[derive(Debug, Clone)]
pub enum MessageScope {
    /// Send to all connections for a specific user
    User { user_id: String },
    /// Send to all connected users
    Broadcast,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pins the action event wire shapes consumers depend on (entity-in-payload).
    #[test]
    fn action_events_serialize_to_expected_wire_shape() {
        let created = Event::ActionCreated {
            coaching_session_id: "sess-1".to_string(),
            action: serde_json::json!({ "id": "act-1", "body": "x" }),
        };
        assert_eq!(
            serde_json::to_value(&created).unwrap(),
            serde_json::json!({
                "type": "action_created",
                "data": { "coaching_session_id": "sess-1", "action": { "id": "act-1", "body": "x" } }
            })
        );

        let deleted = Event::ActionDeleted {
            coaching_session_id: "sess-1".to_string(),
            action_id: "act-1".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&deleted).unwrap(),
            serde_json::json!({
                "type": "action_deleted",
                "data": { "coaching_session_id": "sess-1", "action_id": "act-1" }
            })
        );
        assert_eq!(created.event_type(), "action_created");
        assert_eq!(deleted.event_type(), "action_deleted");
    }
}
