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

    // Overarching Goals (relationship-scoped)
    #[serde(rename = "overarching_goal_created")]
    OverarchingGoalCreated {
        coaching_relationship_id: String,
        overarching_goal: Value,
    },
    #[serde(rename = "overarching_goal_updated")]
    OverarchingGoalUpdated {
        coaching_relationship_id: String,
        overarching_goal: Value,
    },
    #[serde(rename = "overarching_goal_deleted")]
    OverarchingGoalDeleted {
        coaching_relationship_id: String,
        overarching_goal_id: String,
    },

    // System events
    #[serde(rename = "force_logout")]
    ForceLogout { reason: String },
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
            Event::OverarchingGoalCreated { .. } => "overarching_goal_created",
            Event::OverarchingGoalUpdated { .. } => "overarching_goal_updated",
            Event::OverarchingGoalDeleted { .. } => "overarching_goal_deleted",
            Event::ForceLogout { .. } => "force_logout",
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
