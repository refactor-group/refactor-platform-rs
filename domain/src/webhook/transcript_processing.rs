use log::debug;

pub fn handle(transcript_id: &str) {
    debug!(
        "transcript.processing: informational event for external_id={} — no action taken",
        transcript_id
    );
}
