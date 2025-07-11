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

use log::*;
use service::{config::Config, logging::Logger, AppState};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let config = get_config();
    Logger::init_logger(&config);

    info!("Starting up...");

    let db_conn = Arc::new(
        service::init_database(config.database_url())
            .await
            .map_err(|e| panic!("Failed to establish DBConnection: {:?}", e.to_string()))
            .unwrap_or_default(),
    );

    if db_conn.ping().await.is_err() {
        panic!("Failed to establish a useable DBConnection and ping the DB successfully.");
    }

    let app_state = AppState::new(config, &db_conn);

    web::init_server(app_state).await.unwrap();
}

fn get_config() -> Config {
    Config::new()
}

// This is the parent test "runner" that initiates all other crate
// unit/integration tests.
#[cfg(test)]
mod all_tests {
    use log::LevelFilter;
    use service::logging::Logger;
    use simplelog::{error, info};
    use std::process::Command;

    #[tokio::test]
    async fn main() {
        let mut config = crate::get_config();
        config.log_level_filter = LevelFilter::Trace;
        Logger::init_logger(&config);

        let mut exit_codes = Vec::new();

        for crate_name in crates_to_test().iter() {
            let mut command = Command::new("cargo");

            info!("<b>Running tests for {:?} crate</b>\r\n", crate_name);

            // It may be that we need to map each crate with specific commands at some point
            // for now calling "--features mock" for each crate.
            command
                .args(["test", "--features", "mock"])
                .args(["-p", crate_name]);

            let output = command.output().unwrap();

            match output.status.success() {
                true => {
                    info!("<b>All {:?} tests completed successfully.\r\n", crate_name)
                }
                false => error!(
                    "<b>{:?} tests completed with errors ({})</b>\r\n",
                    crate_name, output.status
                ),
            }

            info!("{}", String::from_utf8_lossy(output.stdout.as_slice()));
            info!("{}", String::from_utf8_lossy(output.stderr.as_slice()));

            exit_codes.push(output.status.code().unwrap());
        }
        if exit_codes.iter().any(|code| *code != 0i32) {
            error!("** One or more crate tests failed.");
            // Will fail CI
            std::process::exit(1);
        }
        // Will pass CI
        std::process::exit(0);

        fn crates_to_test() -> Vec<String> {
            vec!["entity_api".to_string(), "web".to_string(), "domain".to_string()]
        }
    }
}
