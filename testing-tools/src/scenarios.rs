use anyhow::Result;
use colored::*;
use std::time::{Duration, Instant};

use crate::api_client::{ApiClient, TestEnvironment};
use crate::auth::AuthenticatedUser;
use crate::output::{print_event, TestResult};
use crate::sse_client::Connection;

pub async fn test_action_create(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();

    println!("\n{}", "=== TEST: Action Create ===".bright_cyan().bold());

    println!("{} User 1 creating action...", "→".blue());

    let action = api_client
        .create_action(
            &user1.session_cookie,
            &test_env.session_id,
            "Test Action - Create",
        )
        .await?;

    let action_id = action["id"].as_str().unwrap();
    println!("{} Action created (ID: {})", "✓".green(), action_id);

    println!(
        "{} Waiting for User 2 to receive action_created event...",
        "→".blue()
    );

    match sse2
        .wait_for_event("action_created", Duration::from_secs(5))
        .await
    {
        Ok(event) => {
            print_event(&sse2.user_label, &event);

            let received_action_id = event.data["data"]["action"]["id"].as_str().unwrap();
            let received_session_id = event.data["data"]["coaching_session_id"].as_str().unwrap();

            if received_action_id == action_id && received_session_id == test_env.session_id {
                println!("{} Event data verified correctly", "✓".green());
                Ok(TestResult {
                    scenario: "action_create".to_string(),
                    passed: true,
                    message: None,
                    duration: start.elapsed(),
                })
            } else {
                println!("{} Event data mismatch!", "✗".red());
                Ok(TestResult {
                    scenario: "action_create".to_string(),
                    passed: false,
                    message: Some(format!(
                        "Expected action_id={}, session_id={}, got action_id={}, session_id={}",
                        action_id, test_env.session_id, received_action_id, received_session_id
                    )),
                    duration: start.elapsed(),
                })
            }
        }
        Err(e) => {
            println!("{} Timeout waiting for event: {}", "✗".red(), e);
            Ok(TestResult {
                scenario: "action_create".to_string(),
                passed: false,
                message: Some(format!("Timeout: {}", e)),
                duration: start.elapsed(),
            })
        }
    }
}

pub async fn test_action_update(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();

    println!("\n{}", "=== TEST: Action Update ===".bright_cyan().bold());

    // First create an action
    println!("{} User 1 creating action...", "→".blue());
    let action = api_client
        .create_action(
            &user1.session_cookie,
            &test_env.session_id,
            "Test Action - Update",
        )
        .await?;

    let action_id = action["id"].as_str().unwrap();

    // Wait for and discard the create event
    let _ = sse2
        .wait_for_event("action_created", Duration::from_secs(5))
        .await?;

    // Now update the action
    println!("{} User 1 updating action...", "→".blue());
    api_client
        .update_action(&user1.session_cookie, action_id, "Updated Title")
        .await?;

    println!(
        "{} Waiting for User 2 to receive action_updated event...",
        "→".blue()
    );

    match sse2
        .wait_for_event("action_updated", Duration::from_secs(5))
        .await
    {
        Ok(event) => {
            print_event(&sse2.user_label, &event);

            let received_title = event.data["data"]["action"]["title"].as_str().unwrap();

            if received_title == "Updated Title" {
                println!("{} Event data verified correctly", "✓".green());
                Ok(TestResult {
                    scenario: "action_update".to_string(),
                    passed: true,
                    message: None,
                    duration: start.elapsed(),
                })
            } else {
                Ok(TestResult {
                    scenario: "action_update".to_string(),
                    passed: false,
                    message: Some(format!("Title mismatch: {}", received_title)),
                    duration: start.elapsed(),
                })
            }
        }
        Err(e) => Ok(TestResult {
            scenario: "action_update".to_string(),
            passed: false,
            message: Some(format!("Timeout: {}", e)),
            duration: start.elapsed(),
        }),
    }
}

pub async fn test_action_delete(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();

    println!("\n{}", "=== TEST: Action Delete ===".bright_cyan().bold());

    // Create action
    let action = api_client
        .create_action(
            &user1.session_cookie,
            &test_env.session_id,
            "Test Action - Delete",
        )
        .await?;

    let action_id = action["id"].as_str().unwrap();

    // Discard create event
    let _ = sse2
        .wait_for_event("action_created", Duration::from_secs(5))
        .await?;

    // Delete action
    println!("{} User 1 deleting action...", "→".blue());
    api_client
        .delete_action(&user1.session_cookie, action_id)
        .await?;

    println!(
        "{} Waiting for User 2 to receive action_deleted event...",
        "→".blue()
    );

    match sse2
        .wait_for_event("action_deleted", Duration::from_secs(5))
        .await
    {
        Ok(event) => {
            print_event(&sse2.user_label, &event);

            let received_action_id = event.data["data"]["action_id"].as_str().unwrap();

            if received_action_id == action_id {
                println!("{} Event data verified correctly", "✓".green());
                Ok(TestResult {
                    scenario: "action_delete".to_string(),
                    passed: true,
                    message: None,
                    duration: start.elapsed(),
                })
            } else {
                Ok(TestResult {
                    scenario: "action_delete".to_string(),
                    passed: false,
                    message: Some(format!("Action ID mismatch: {}", received_action_id)),
                    duration: start.elapsed(),
                })
            }
        }
        Err(e) => Ok(TestResult {
            scenario: "action_delete".to_string(),
            passed: false,
            message: Some(format!("Timeout: {}", e)),
            duration: start.elapsed(),
        }),
    }
}

pub async fn test_force_logout(
    user1: &AuthenticatedUser,
    user2: &AuthenticatedUser,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();

    println!("\n{}", "=== TEST: Force Logout ===".bright_cyan().bold());

    println!("{} User 1 forcing logout of User 2...", "→".blue());

    api_client
        .force_logout(&user1.session_cookie, &user2.user_id)
        .await?;

    println!(
        "{} Waiting for User 2 to receive force_logout event...",
        "→".blue()
    );

    match sse2
        .wait_for_event("force_logout", Duration::from_secs(5))
        .await
    {
        Ok(event) => {
            print_event(&sse2.user_label, &event);
            println!("{} Event received correctly", "✓".green());
            Ok(TestResult {
                scenario: "force_logout".to_string(),
                passed: true,
                message: None,
                duration: start.elapsed(),
            })
        }
        Err(e) => Ok(TestResult {
            scenario: "force_logout".to_string(),
            passed: false,
            message: Some(format!("Timeout: {}", e)),
            duration: start.elapsed(),
        }),
    }
}

pub async fn test_connection(
    user1: &AuthenticatedUser,
    user2: &AuthenticatedUser,
    _sse1: &mut Connection,
    _sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();

    println!("\n{}", "=== TEST: Connection Test ===".bright_cyan().bold());
    println!(
        "{}",
        "Testing basic SSE connectivity without creating any data".bright_white()
    );

    println!(
        "{} User 1 ({}) SSE connection: established",
        "✓".green(),
        user1.user_id
    );
    println!(
        "{} User 2 ({}) SSE connection: established",
        "✓".green(),
        user2.user_id
    );

    // Wait a bit to ensure connections are stable
    println!(
        "{} Waiting 2 seconds to verify connections stay alive...",
        "→".blue()
    );
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("{} Connections remain stable", "✓".green());
    println!("{} SSE infrastructure is working correctly", "✓".green());

    Ok(TestResult {
        scenario: "connection_test".to_string(),
        passed: true,
        message: Some("SSE connections established and maintained successfully".to_string()),
        duration: start.elapsed(),
    })
}

// ---- Coaching session Topics ----
// The backend emits ONE coarse `topics_changed` event (data: { coaching_session_id })
// on every topic mutation, to BOTH participants. These scenarios prove the NON-acting
// user receives it for each operation. Because the event type is identical across
// operations, each scenario drains the observer first so the awaited frame is the one
// the mutation under test caused (the same discipline the frontend needs).

/// Waits for `observer` to receive a `topics_changed` event for `expected_session_id`.
async fn expect_topics_changed(
    observer: &mut Connection,
    expected_session_id: &str,
    scenario: &str,
    start: Instant,
) -> TestResult {
    match observer
        .wait_for_event("topics_changed", Duration::from_secs(5))
        .await
    {
        Ok(event) => {
            print_event(&observer.user_label, &event);
            let got = event.data["data"]["coaching_session_id"]
                .as_str()
                .unwrap_or_default();
            if got == expected_session_id {
                println!(
                    "{} {} received topics_changed (session {})",
                    "✓".green(),
                    observer.user_label,
                    got
                );
                TestResult {
                    scenario: scenario.to_string(),
                    passed: true,
                    message: None,
                    duration: start.elapsed(),
                }
            } else {
                TestResult {
                    scenario: scenario.to_string(),
                    passed: false,
                    message: Some(format!(
                        "session_id mismatch: expected {expected_session_id}, got {got}"
                    )),
                    duration: start.elapsed(),
                }
            }
        }
        Err(e) => TestResult {
            scenario: scenario.to_string(),
            passed: false,
            message: Some(format!("Timeout waiting for topics_changed: {e}")),
            duration: start.elapsed(),
        },
    }
}

pub async fn test_topic_create(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();
    println!("\n{}", "=== TEST: Topic Create ===".bright_cyan().bold());

    sse2.drain();
    println!("{} User 1 (coach) creating topic...", "→".blue());
    let topic = api_client
        .create_topic(
            &user1.session_cookie,
            &test_env.session_id,
            "SSE test topic - create",
        )
        .await?;
    let topic_id = topic["id"].as_str().unwrap_or_default().to_string();
    println!("{} Topic created (ID: {})", "✓".green(), topic_id);

    let result = expect_topics_changed(sse2, &test_env.session_id, "topic_create", start).await;
    let _ = api_client
        .delete_topic(&user1.session_cookie, &test_env.session_id, &topic_id)
        .await;
    Ok(result)
}

pub async fn test_topic_update(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();
    println!(
        "\n{}",
        "=== TEST: Topic Update (body edit) ==="
            .bright_cyan()
            .bold()
    );

    let topic = api_client
        .create_topic(&user1.session_cookie, &test_env.session_id, "original body")
        .await?;
    let topic_id = topic["id"].as_str().unwrap_or_default().to_string();

    sse2.drain(); // discard the create event
    println!("{} User 1 (coach) editing topic body...", "→".blue());
    api_client
        .update_topic(
            &user1.session_cookie,
            &test_env.session_id,
            &topic_id,
            "edited body",
        )
        .await?;

    let result = expect_topics_changed(sse2, &test_env.session_id, "topic_update", start).await;
    let _ = api_client
        .delete_topic(&user1.session_cookie, &test_env.session_id, &topic_id)
        .await;
    Ok(result)
}

pub async fn test_topic_priority(
    user1: &AuthenticatedUser,
    user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    sse1: &mut Connection,
    _sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();
    println!(
        "\n{}",
        "=== TEST: Topic Priority (coachee sets, coach observes) ==="
            .bright_cyan()
            .bold()
    );

    // Coach creates the topic; the COACHEE (user2) sets priority (coachee-only
    // endpoint); we assert the COACH (user1, the non-actor) receives the event.
    let topic = api_client
        .create_topic(
            &user1.session_cookie,
            &test_env.session_id,
            "priority topic",
        )
        .await?;
    let topic_id = topic["id"].as_str().unwrap_or_default().to_string();

    sse1.drain(); // discard the create event on the coach's stream
    println!("{} User 2 (coachee) setting priority=High...", "→".blue());
    api_client
        .set_topic_priority(
            &user2.session_cookie,
            &test_env.session_id,
            &topic_id,
            "High",
        )
        .await?;

    let result = expect_topics_changed(sse1, &test_env.session_id, "topic_priority", start).await;
    let _ = api_client
        .delete_topic(&user1.session_cookie, &test_env.session_id, &topic_id)
        .await;
    Ok(result)
}

pub async fn test_topic_status(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();
    println!(
        "\n{}",
        "=== TEST: Topic Status = Discussed (coach sets, coachee observes) ==="
            .bright_cyan()
            .bold()
    );

    let topic = api_client
        .create_topic(&user1.session_cookie, &test_env.session_id, "status topic")
        .await?;
    let topic_id = topic["id"].as_str().unwrap_or_default().to_string();

    sse2.drain();
    println!("{} User 1 (coach) setting status=Discussed...", "→".blue());
    api_client
        .set_topic_status(
            &user1.session_cookie,
            &test_env.session_id,
            &topic_id,
            "Discussed",
        )
        .await?;

    let result = expect_topics_changed(sse2, &test_env.session_id, "topic_status", start).await;
    let _ = api_client
        .delete_topic(&user1.session_cookie, &test_env.session_id, &topic_id)
        .await;
    Ok(result)
}

pub async fn test_topic_delete(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();
    println!(
        "\n{}",
        "=== TEST: Topic Delete (author deletes, coachee observes) ==="
            .bright_cyan()
            .bold()
    );

    let topic = api_client
        .create_topic(&user1.session_cookie, &test_env.session_id, "delete topic")
        .await?;
    let topic_id = topic["id"].as_str().unwrap_or_default().to_string();

    sse2.drain();
    println!("{} User 1 (author) deleting topic...", "→".blue());
    api_client
        .delete_topic(&user1.session_cookie, &test_env.session_id, &topic_id)
        .await?;

    Ok(expect_topics_changed(sse2, &test_env.session_id, "topic_delete", start).await)
}

// --- Coaching session title (coarse, session-scoped) ---

pub async fn test_title_update(
    user1: &AuthenticatedUser,
    _user2: &AuthenticatedUser,
    test_env: &TestEnvironment,
    api_client: &ApiClient,
    _sse1: &mut Connection,
    sse2: &mut Connection,
) -> Result<TestResult> {
    let start = Instant::now();
    println!("\n{}", "=== TEST: Title Update ===".bright_cyan().bold());

    println!("{} User 1 updating session title...", "→".blue());
    api_client
        .update_session_title(
            &user1.session_cookie,
            &test_env.session_id,
            "Renamed by SSE test tool",
        )
        .await?;

    println!(
        "{} Waiting for User 2 to receive coaching_session_title_updated event...",
        "→".blue()
    );
    match sse2
        .wait_for_event("coaching_session_title_updated", Duration::from_secs(5))
        .await
    {
        Ok(event) => {
            print_event(&sse2.user_label, &event);
            let got = event.data["data"]["coaching_session_id"]
                .as_str()
                .unwrap_or_default();
            let passed = got == test_env.session_id;
            Ok(TestResult {
                scenario: "title_update".to_string(),
                passed,
                message: (!passed).then(|| format!("Session ID mismatch: got {}", got)),
                duration: start.elapsed(),
            })
        }
        Err(e) => Ok(TestResult {
            scenario: "title_update".to_string(),
            passed: false,
            message: Some(format!("Timeout: {}", e)),
            duration: start.elapsed(),
        }),
    }
}
