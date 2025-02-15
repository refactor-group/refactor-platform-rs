use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TiptapCollabClaims {
    pub(crate) exp: usize,
    pub(crate) iss: String,
    pub(crate) sub: String,
    // Titap requires this claim to be JS style case.
    #[serde(rename = "allowedDocumentNames")]
    pub(crate) allowed_document_names: Vec<String>,
}
