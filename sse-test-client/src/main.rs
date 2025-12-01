use anyhow::Result;
use clap::Parser;
use colored::*;

mod api_client;
mod auth;
mod output;
mod scenarios;
mod sse_client;

use api_client::ApiClient;
use auth::{login, UserCredentials};
use output::print_test_summary;
use sse_client::Connection;

#[derive(Parser)]
#[command(name = "sse-test-client")]
#[command(about = "SSE Integration Testing Tool")]
struct Cli {
    /// Base URL of the backend (e.g., http://localhost:4747)
    #[arg(long)]
    base_url: String,

    /// User 1 credentials (format: email:password)
    #[arg(long)]
    user1: String,

    /// User 2 credentials (format: email:password)
    #[arg(long)]
    user2: String,

    /// Test scenario to run
    #[arg(long, value_enum)]
    scenario: ScenarioChoice,

    /// Enable verbose output
    #[arg(long, short)]
    verbose: bool,
}

#[derive(clap::ValueEnum, Clone)]
enum ScenarioChoice {
    /// Test basic SSE connection without creating any data
    ConnectionTest,
    /// Test force logout event (no coaching data needed)
    ForceLogoutTest,
    /// Test action create event (requires coaching session)
    ActionCreate,
    /// Test action update event (requires coaching session)
    ActionUpdate,
    /// Test action delete event (requires coaching session)
    ActionDelete,
    /// Run all tests including those requiring coaching data
    All,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    }

    println!("{}", "=== SETUP PHASE ===".bright_white().bold());

    // Parse credentials
    let user1_creds = UserCredentials::parse(&cli.user1)?;
    let user2_creds = UserCredentials::parse(&cli.user2)?;

    // Authenticate users
    println!("{} Authenticating users...", "→".blue());
    let client = reqwest::Client::new();
    let user1 = login(&client, &cli.base_url, &user1_creds).await?;
    let user2 = login(&client, &cli.base_url, &user2_creds).await?;

    println!(
        "{} User 1 authenticated (ID: {})",
        "✓".green(),
        user1.user_id
    );
    println!(
        "{} User 2 authenticated (ID: {})",
        "✓".green(),
        user2.user_id
    );

    // Set up test environment only for scenarios that need coaching data
    let api_client = ApiClient::new(client.clone(), cli.base_url.clone());
    let test_env = match cli.scenario {
        ScenarioChoice::ConnectionTest | ScenarioChoice::ForceLogoutTest => {
            println!(
                "\n{} Skipping test environment setup (not needed for this test)",
                "→".blue()
            );
            None
        }
        _ => {
            println!(
                "\n{} Creating test coaching relationship and session...",
                "→".blue()
            );
            let env = api_client
                .setup_test_environment(
                    &user1.session_cookie,
                    &user2.session_cookie,
                    &user1.user_id,
                    &user2.user_id,
                )
                .await?;

            println!(
                "{} Coaching relationship created (ID: {})",
                "✓".green(),
                env.relationship_id
            );
            println!(
                "{} Coaching session created (ID: {})",
                "✓".green(),
                env.session_id
            );
            Some(env)
        }
    };

    // Establish SSE connections
    println!("\n{} Establishing SSE connections...", "→".blue());
    let mut sse1 = Connection::establish(
        &cli.base_url,
        &user1.session_cookie,
        "User 1 (Coach)".to_string(),
    )
    .await?;

    let mut sse2 = Connection::establish(
        &cli.base_url,
        &user2.session_cookie,
        "User 2 (Coachee)".to_string(),
    )
    .await?;

    println!("{} User 1 SSE connection established", "✓".green());
    println!("{} User 2 SSE connection established", "✓".green());

    // Run test scenarios
    println!("\n{}", "=== TEST PHASE ===".bright_white().bold());

    let mut results = Vec::new();

    match cli.scenario {
        ScenarioChoice::ConnectionTest => {
            results.push(scenarios::test_connection(&user1, &user2, &mut sse1, &mut sse2).await?);
        }
        ScenarioChoice::ForceLogoutTest => {
            results.push(
                scenarios::test_force_logout(&user1, &user2, &api_client, &mut sse1, &mut sse2)
                    .await?,
            );
        }
        ScenarioChoice::ActionCreate => {
            let env = test_env
                .as_ref()
                .expect("Test environment required for ActionCreate");
            results.push(
                scenarios::test_action_create(
                    &user1,
                    &user2,
                    env,
                    &api_client,
                    &mut sse1,
                    &mut sse2,
                )
                .await?,
            );
        }
        ScenarioChoice::ActionUpdate => {
            let env = test_env
                .as_ref()
                .expect("Test environment required for ActionUpdate");
            results.push(
                scenarios::test_action_update(
                    &user1,
                    &user2,
                    env,
                    &api_client,
                    &mut sse1,
                    &mut sse2,
                )
                .await?,
            );
        }
        ScenarioChoice::ActionDelete => {
            let env = test_env
                .as_ref()
                .expect("Test environment required for ActionDelete");
            results.push(
                scenarios::test_action_delete(
                    &user1,
                    &user2,
                    env,
                    &api_client,
                    &mut sse1,
                    &mut sse2,
                )
                .await?,
            );
        }
        ScenarioChoice::All => {
            results.push(scenarios::test_connection(&user1, &user2, &mut sse1, &mut sse2).await?);
            results.push(
                scenarios::test_force_logout(&user1, &user2, &api_client, &mut sse1, &mut sse2)
                    .await?,
            );
            let env = test_env
                .as_ref()
                .expect("Test environment required for All scenarios");
            results.push(
                scenarios::test_action_create(
                    &user1,
                    &user2,
                    env,
                    &api_client,
                    &mut sse1,
                    &mut sse2,
                )
                .await?,
            );
            results.push(
                scenarios::test_action_update(
                    &user1,
                    &user2,
                    env,
                    &api_client,
                    &mut sse1,
                    &mut sse2,
                )
                .await?,
            );
            results.push(
                scenarios::test_action_delete(
                    &user1,
                    &user2,
                    env,
                    &api_client,
                    &mut sse1,
                    &mut sse2,
                )
                .await?,
            );
        }
    }

    // Print summary
    println!("\n{}", "=== RESULTS ===".bright_white().bold());
    print_test_summary(&results);

    let all_passed = results.iter().all(|r| r.passed);

    if all_passed {
        println!("\n{}", "All tests passed! ✓".bright_green().bold());
    } else {
        println!("\n{}", "Some tests failed! ✗".bright_red().bold());
    }

    std::process::exit(if all_passed { 0 } else { 1 });
}
