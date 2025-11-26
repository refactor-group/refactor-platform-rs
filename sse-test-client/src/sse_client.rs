use anyhow::Result;
use eventsource_client::{self as es, Client};
use futures_util::stream::StreamExt;
use log::*;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: String,
    pub data: Value,
    pub timestamp: Instant,
}

pub struct Connection {
    pub user_label: String,
    event_rx: mpsc::UnboundedReceiver<Event>,
    _handle: tokio::task::JoinHandle<()>,
}

impl Connection {
    pub async fn establish(
        base_url: &str,
        session_cookie: &str,
        user_label: String,
    ) -> Result<Self> {
        let url = format!("{}/sse", base_url);
        let (tx, rx) = mpsc::unbounded_channel();

        let client = es::ClientBuilder::for_url(&url)?
            .header("Cookie", &format!("id={}", session_cookie))?
            .build();

        let label = user_label.clone();
        let handle = tokio::spawn(async move {
            let mut stream = client.stream();

            loop {
                match stream.next().await {
                    Some(Ok(es::SSE::Event(event))) => {
                        if let Ok(data) = serde_json::from_str(&event.data) {
                            let sse_event = Event {
                                event_type: event.event_type,
                                data,
                                timestamp: Instant::now(),
                            };

                            if tx.send(sse_event).is_err() {
                                debug!("SSE receiver dropped for {}", label);
                                break;
                            }
                        }
                    }
                    Some(Ok(es::SSE::Comment(_))) => {
                        // Ignore comments (keep-alive)
                    }
                    Some(Err(e)) => {
                        warn!("SSE error for {}: {}", label, e);
                    }
                    None => {
                        debug!("SSE stream ended for {}", label);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            user_label,
            event_rx: rx,
            _handle: handle,
        })
    }

    pub async fn wait_for_event(&mut self, event_type: &str, timeout: Duration) -> Result<Event> {
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("Timeout waiting for event: {}", event_type);
            }

            match tokio::time::timeout(remaining, self.event_rx.recv()).await {
                Ok(Some(event)) if event.event_type == event_type => {
                    return Ok(event);
                }
                Ok(Some(_)) => {
                    // Wrong event type, keep waiting
                    continue;
                }
                Ok(None) => {
                    anyhow::bail!("SSE connection closed");
                }
                Err(_) => {
                    anyhow::bail!("Timeout waiting for event: {}", event_type);
                }
            }
        }
    }
}
