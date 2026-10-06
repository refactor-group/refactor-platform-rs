//! Google Meet API client for creating meeting spaces and finding who attended a conference.
//!
//! This module provides an HTTP client for the Google Meet API: creating meeting spaces, and
//! listing the participants of the conference that overlapped a recording.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use chrono::{DateTime, Utc};
use log::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

const MEET_HOST: &str = "meet.google.com";

/// Google Meet space configuration
#[derive(Debug, Serialize)]
pub struct SpaceConfig {
    #[serde(rename = "accessType")]
    pub access_type: String,
}

/// Request to create a Google Meet space
#[derive(Debug, Serialize)]
pub struct CreateSpaceRequest {
    pub config: SpaceConfig,
}

/// Response from creating a Google Meet space
#[derive(Debug, Deserialize)]
pub struct SpaceResponse {
    pub name: String,
    #[serde(rename = "meetingUri")]
    pub meeting_uri: String,
    #[serde(rename = "meetingCode")]
    pub meeting_code: String,
}

/// One attendee of a Meet conference. `user` is `users/<id>` for a signed-in Google account and
/// `None` for anonymous or phone attendees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub user: Option<String>,
    pub display_name: Option<String>,
}

/// One conference (a single occurrence of a meeting) held in a Meet space.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConferenceRecord {
    name: String,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConferenceRecordsPage {
    #[serde(default)]
    conference_records: Vec<ConferenceRecord>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParticipantsPage {
    #[serde(default)]
    participants: Vec<ParticipantEntry>,
    next_page_token: Option<String>,
}

/// Exactly one of the user fields is present per participant.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParticipantEntry {
    signedin_user: Option<SignedInUser>,
    anonymous_user: Option<NamedUser>,
    phone_user: Option<NamedUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedInUser {
    user: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamedUser {
    display_name: Option<String>,
}

impl From<ParticipantEntry> for Participant {
    fn from(entry: ParticipantEntry) -> Self {
        match (entry.signedin_user, entry.anonymous_user, entry.phone_user) {
            (Some(signed_in), _, _) => Self {
                user: signed_in.user,
                display_name: signed_in.display_name,
            },
            (None, anonymous, phone) => Self {
                user: None,
                display_name: anonymous.or(phone).and_then(|u| u.display_name),
            },
        }
    }
}

/// The meeting code from a `meet.google.com` URL, or `None` for any other URL.
pub fn meeting_code_from_url(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let (host, path) = rest.split_once('/')?;
    let code = path.split(['/', '?', '#']).next()?;
    (host == MEET_HOST && !code.is_empty()).then(|| code.to_string())
}

/// The single record overlapping the recording window, or `None` when zero or several do.
fn overlapping_record(
    records: Vec<ConferenceRecord>,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Option<ConferenceRecord> {
    let mut overlapping = records
        .into_iter()
        .filter(|r| r.start_time <= ended_at && r.end_time.is_none_or(|end| end >= started_at));
    overlapping.next().filter(|_| overlapping.next().is_none())
}

/// Google Meet API client
pub struct Client {
    client: reqwest::Client,
    base_url: String,
}

impl Client {
    /// Create a new Google Meet client with the given access token and base URL
    pub fn new(access_token: &str, base_url: &str) -> Result<Self, Error> {
        let mut headers = reqwest::header::HeaderMap::new();

        let auth_value = format!("Bearer {}", access_token);
        let mut header_value =
            reqwest::header::HeaderValue::from_str(&auth_value).map_err(|e| {
                warn!("Failed to create auth header: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                        "Invalid access token format".to_string(),
                    )),
                }
            })?;
        header_value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, header_value);

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            base_url: base_url.to_string(),
        })
    }

    /// Create a new Google Meet space
    pub async fn create_space(&self) -> Result<SpaceResponse, Error> {
        let url = format!("{}/spaces", self.base_url);

        let request = CreateSpaceRequest {
            config: SpaceConfig {
                access_type: "TRUSTED".to_string(),
            },
        };

        debug!("Creating Google Meet space");

        let space: SpaceResponse = self
            .send_json(self.client.post(&url).json(&request))
            .await?;
        info!("Created Google Meet space: {}", space.meeting_code);
        Ok(space)
    }

    /// Participants of the one conference in the meeting's space that overlapped the recording.
    ///
    /// Returns `Ok(None)` when no conference, or more than one, overlaps the window.
    pub async fn conference_participants(
        &self,
        meeting_code: &str,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
    ) -> Result<Option<Vec<Participant>>, Error> {
        let records = self.list_conference_records(meeting_code).await?;
        let Some(record) = overlapping_record(records, started_at, ended_at) else {
            return Ok(None);
        };
        self.list_participants(&record.name).await.map(Some)
    }

    async fn list_conference_records(
        &self,
        meeting_code: &str,
    ) -> Result<Vec<ConferenceRecord>, Error> {
        let url = format!("{}/conferenceRecords", self.base_url);
        let filter = format!("space.meeting_code=\"{meeting_code}\"");
        let mut records = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            debug!("Listing Google Meet conference records");
            let mut request = self.client.get(&url).query(&[("filter", &filter)]);
            if let Some(token) = &page_token {
                request = request.query(&[("pageToken", token)]);
            }
            let page: ConferenceRecordsPage = self.send_json(request).await?;
            records.extend(page.conference_records);
            page_token = page.next_page_token.filter(|t| !t.is_empty());
            if page_token.is_none() {
                return Ok(records);
            }
        }
    }

    async fn list_participants(&self, record_name: &str) -> Result<Vec<Participant>, Error> {
        let url = format!("{}/{}/participants", self.base_url, record_name);
        let mut participants = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            debug!("Listing Google Meet conference participants");
            let mut request = self.client.get(&url);
            if let Some(token) = &page_token {
                request = request.query(&[("pageToken", token)]);
            }
            let page: ParticipantsPage = self.send_json(request).await?;
            participants.extend(page.participants.into_iter().map(Participant::from));
            page_token = page.next_page_token.filter(|t| !t.is_empty());
            if page_token.is_none() {
                return Ok(participants);
            }
        }
    }

    /// Send a request and decode a JSON success body, mapping failures to external errors.
    async fn send_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, Error> {
        let response = request.send().await.map_err(|e| {
            warn!("Failed to reach Google Meet API: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            response.json().await.map_err(|e| {
                warn!("Failed to parse Google Meet response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Google Meet API".to_string(),
                    )),
                }
            })
        } else if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            warn!("Google Meet API returned 401 Unauthorized: access token expired or revoked");
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::OauthTokenRevoked(
                    "google".to_string(),
                )),
            })
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google Meet API error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }
}

#[cfg(test)]
#[path = "google_meet_tests.rs"]
mod tests;
