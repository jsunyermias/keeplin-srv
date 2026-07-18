// md:Overview
use anyhow::Context;
use keeplin_srv::{config::Config, crypto::Cipher, reencrypt};

// md:fn parse_args
fn parse_args() -> anyhow::Result<reencrypt::Options> {
    let mut opts = reencrypt::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => opts.dry_run = true,
            "--batch-size" => {
                let value = args
                    .next()
                    .context("--batch-size requires a value")?
                    .parse::<i64>()
                    .context("--batch-size must be a positive integer")?;
                anyhow::ensure!(value > 0, "--batch-size must be positive");
                opts.batch_size = value;
            }
            "--help" | "-h" => {
                println!("Usage: keeplin-reencrypt [--dry-run] [--batch-size N]");
                println!();
                println!("Rewrites plaintext notes.title / lines.content rows to the");
                println!("enc:v1: at-rest-encrypted form. Requires DATABASE_URL and");
                println!("AT_REST_KEY in the environment (same Config as the server).");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(opts)
}

// md:fn main
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("keeplin_srv=info".parse()?),
        )
        .init();

    let opts = parse_args()?;
    let config = Config::from_env();
    let cipher = Cipher::from_key(config.at_rest_key.as_deref())
        .map_err(|e| anyhow::anyhow!("AT_REST_KEY: {e}"))?;
    anyhow::ensure!(
        cipher.enabled(),
        "AT_REST_KEY must be set: without it there is nothing to re-encrypt to"
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&config.database_url)
        .await
        .context("connect to PostgreSQL")?;

    let stats = reencrypt::run(&pool, &cipher, &opts)
        .await
        .map_err(|e| anyhow::anyhow!("re-encrypt pass failed: {e}"))?;

    let mode = if opts.dry_run { "DRY RUN — " } else { "" };
    println!(
        "{mode}notes.title: {} plaintext row(s) found, {} rewritten, {} skipped (concurrent)",
        stats.notes_title.scanned,
        stats.notes_title.rewritten,
        stats.notes_title.skipped_concurrent
    );
    println!(
        "{mode}lines.content: {} plaintext row(s) found, {} rewritten, {} skipped (concurrent)",
        stats.lines_content.scanned,
        stats.lines_content.rewritten,
        stats.lines_content.skipped_concurrent
    );
    Ok(())
}
