// Copyright (c) 2023 Jim Hodapp & Caleb Bourg
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.
//!
//! **Refactor Coaching Platform**
//!
//! A Rust-based backend that provides a web API for various client applications
//! (e.g. a web frontend) that facilitate the coaching of software engineers.
//!
//! The platform itself is useful for professional independent coaches, informal
//! mentors and engineering leaders who work with individual software engineers
//! and/or teams by providing a single application that facilitates and enhances
//! your coaching practice.

use domain::gateway::{object_storage, recall_ai};
use events::EventPublisher;
use log::*;
use meeting_ai::traits::{recording_bot, transcription as transcription_trait};
use meeting_auth::webhook::svix::Validator as SvixValidator;
use service::{config::Config, logging::Logger};
use std::process;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    service::load_env_file();
    let config = Config::new();
    Logger::init_logger(&config);

    info!("Starting up...");

    let db_conn = match service::init_database(&config).await {
        Ok(db) => Arc::new(db),
        Err(e) => {
            error!("Failed to establish database connection: {e}");
            process::exit(1);
        }
    };

    if db_conn.ping().await.is_err() {
        error!("Failed to ping the database after establishing connection");
        process::exit(1);
    }

    // Fail fast on a malformed secret rather than returning 401 on the first webhook.
    if let Some(secret) = config.recall_ai_webhook_secret() {
        if let Err(e) = SvixValidator::new("recall_ai".to_string(), &secret) {
            error!("RECALL_AI_WEBHOOK_SECRET is set but invalid: {:?}", e);
            process::exit(1);
        }
    }

    // Create service-level state (infrastructure only - no SSE)
    let service_state = service::AppState::new(config, &db_conn);

    // Create SSE manager (web/application layer concern)
    let sse_manager = Arc::new(sse::Manager::new());

    // Create event publisher and register SSE event handler
    let sse_event_handler = Arc::new(sse::SseDomainEventHandler::new(Arc::clone(&sse_manager)));
    let event_publisher = EventPublisher::new().with_handler(sse_event_handler);

    // Build meeting provider from config. Both bot and transcript traits share the same
    // underlying client so we build one instance and wrap it in two Arc<dyn Trait>s.
    let (recording_bot_provider, transcription_provider) =
        match service_state.config.recall_ai_api_key() {
            Some(key) => {
                match recall_ai::Provider::new(&key, service_state.config.recall_ai_region()) {
                    Ok(p) => {
                        let bot: Arc<dyn recording_bot::Provider> = Arc::new(p.clone());
                        let transcript: Arc<dyn transcription_trait::Provider> = Arc::new(p);
                        (Some(bot), Some(transcript))
                    }
                    Err(e) => {
                        warn!(
                            "Failed to build Recall.ai provider — recording disabled: {:?}",
                            e
                        );
                        (None, None)
                    }
                }
            }
            None => {
                info!("RECALL_AI_API_KEY not set — meeting recording and transcription disabled");
                (None, None)
            }
        };

    let object_store = object_storage::from_config(&service_state.config);
    if object_store.is_none() {
        info!("Object storage is not configured — coaching note image upload and serving disabled");
    }

    // Create web-level state (adds domain and SSE concerns)
    let web_state = web::AppState::new(
        service_state,
        sse_manager,
        event_publisher,
        recording_bot_provider,
        transcription_provider,
        object_store,
    );

    web::init_server(web_state).await.unwrap();
}

// Runs the mock-gated suites so a plain `cargo test` covers them too.
#[cfg(test)]
mod all_tests {
    use std::process::Command;

    #[test]
    fn mock_gated_suites_pass() {
        let status = Command::new(env!("CARGO"))
            .args(["test", "-p", "entity_api", "-p", "domain", "-p", "web"])
            .args(["--features", "domain/mock,web/mock"])
            .status()
            .expect("failed to spawn cargo");

        assert!(status.success(), "mock-gated tests failed: {status}");
    }
}
