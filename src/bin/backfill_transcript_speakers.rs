//! Backfills speaker attribution for transcripts completed before attribution existed.
//!
//! Modes are selected by environment variable, not CLI args, since `Config::new()` rejects
//! unknown argv. A dry run (the default) uses a read-only database session; `BACKFILL_APPLY=1`
//! writes. See `docs/test-plans/transcript_speaker_attribution_backfill.md`.

use std::fs::File;
use std::io::Write;
use std::process;

use domain::gateway::recall_ai;
use domain::transcript_backfill::{self as backfill, Options, Providers, Row, Summary};
use log::info;
use service::{config::Config, logging::Logger};

#[tokio::main]
async fn main() {
    service::load_env_file();
    let config = Config::new();
    Logger::init_logger(&config);

    if let Err(message) = run(&config).await {
        eprintln!("backfill_transcript_speakers: {message}");
        process::exit(1);
    }
}

async fn run(config: &Config) -> Result<(), String> {
    let options = Options::from_lookup(|name| std::env::var(name).ok())?;

    let key = config
        .recall_ai_api_key()
        .ok_or("RECALL_AI_API_KEY is not set; the backfill needs it to re-download transcripts")?;
    let recall = recall_ai::Provider::new(&key, config.recall_ai_region())
        .map_err(|_| "RECALL_AI_API_KEY is not a usable Recall.ai key".to_string())?;
    let providers = Providers {
        transcripts: &recall,
        bot: &recall,
    };

    let db = backfill::connect(config, options.apply)
        .await
        .map_err(|e| format!("opening the database failed: {e}"))?;
    info!(
        "Backfill starting: apply={} google={} limit={:?} since={:?} delay={:?}",
        options.apply, options.google, options.limit, options.since, options.delay
    );

    let transcriptions = backfill::candidates(&db, &options)
        .await
        .map_err(|e| format!("finding candidates failed: {e}"))?;
    info!("Backfill candidates: {}", transcriptions.len());

    let path = backfill::report_file_name();
    let mut report =
        File::create(&path).map_err(|e| format!("creating the report {path} failed: {e}"))?;
    let mut write_line = |line: &str| {
        writeln!(report, "{line}").map_err(|e| format!("writing the report {path} failed: {e}"))
    };
    write_line(backfill::CSV_HEADER)?;

    let mut rows: Vec<Row> = Vec::with_capacity(transcriptions.len());
    for (index, transcription) in transcriptions.iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(options.delay).await;
        }
        let row = backfill::process(&db, &providers, config, transcription, &options).await;
        info!(
            "Backfill {}/{}: transcription {} -> {}",
            index + 1,
            transcriptions.len(),
            row.transcription_id,
            row.outcome.as_str()
        );
        write_line(&row.csv_line())?;
        rows.push(row);
    }

    println!("{}", Summary::from_rows(&rows));
    println!("report: {path}");
    Ok(())
}
