use colored::*;
use std::time::Duration;

use crate::sse_client::Event;

#[derive(Debug)]
pub struct TestResult {
    pub scenario: String,
    pub passed: bool,
    pub message: Option<String>,
    pub duration: Duration,
}

pub fn print_event(user_label: &str, event: &Event) {
    let label_colored = if user_label.contains("User 1") {
        user_label.bright_blue()
    } else {
        user_label.bright_magenta()
    };

    println!(
        "\n[{}] {} event received",
        label_colored.bold(),
        event.event_type.yellow()
    );

    if let Ok(pretty) = serde_json::to_string_pretty(&event.data) {
        println!("   {}", pretty.dimmed());
    }
}

pub fn print_test_summary(results: &[TestResult]) {
    println!("\n{}", "=== TEST SUMMARY ===".bright_white().bold());

    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = total - passed;

    for result in results {
        let status = if result.passed {
            "PASS".green().bold()
        } else {
            "FAIL".red().bold()
        };

        println!("[{}] {} ({:?})", status, result.scenario, result.duration);

        if let Some(msg) = &result.message {
            println!("      {}", msg.dimmed());
        }
    }

    println!(
        "\n{}: {} passed, {} failed",
        "Results".bold(),
        passed.to_string().green(),
        failed.to_string().red()
    );
}
