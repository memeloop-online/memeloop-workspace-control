use std::{
    fs::OpenOptions,
    io::{BufReader, BufWriter},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use clap::{Parser, Subcommand};

use crate::{config::AppConfig, storage::Database};

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(flatten)]
    pub config: AppConfig,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the API server and background workers.
    Serve,
    /// Manage users and other control-plane state.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
    /// Run database management operations.
    Database {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// Create an API user. Pass the high-entropy token through MWC_ADMIN_TOKEN.
    CreateUser {
        #[arg(long)]
        display_name: String,
        #[arg(long, env = "MWC_ADMIN_TOKEN", hide_env_values = true)]
        token: String,
        #[arg(long, default_value_t = false)]
        system_admin: bool,
    },
    /// Generate a new random Base64 installation encryption key.
    GenerateEncryptionKey,
}

#[derive(Debug, Subcommand)]
pub enum DatabaseCommand {
    /// Apply all built-in schema migrations and verify the installation identity.
    Migrate,
    /// Export an encrypted, installation-bound SQLite snapshot to a new file.
    Export {
        #[arg(long)]
        output: PathBuf,
    },
    /// Import a snapshot into an empty PostgreSQL database.
    Import {
        #[arg(long)]
        input: PathBuf,
    },
    /// Copy the configured SQLite database into an empty PostgreSQL database.
    MigrateToPostgres {
        #[arg(long, env = "MWC_DESTINATION_DATABASE_URL", hide_env_values = true)]
        destination_url: String,
    },
}

pub async fn execute_database(
    database: &Database,
    command: DatabaseCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        DatabaseCommand::Migrate => {
            database.migrate().await?;
            println!(
                "database schema version {}",
                database.schema_version().await?
            );
        }
        DatabaseCommand::Export { output } => {
            let snapshot = database.export_snapshot(unix_timestamp()?).await?;
            write_snapshot(&output, &snapshot)?;
            println!("exported encrypted snapshot to {}", output.display());
        }
        DatabaseCommand::Import { input } => {
            let snapshot = read_snapshot(&input)?;
            database.import_snapshot(&snapshot).await?;
            println!("imported snapshot from {}", input.display());
        }
        DatabaseCommand::MigrateToPostgres { destination_url } => {
            let snapshot = database.export_snapshot(unix_timestamp()?).await?;
            let destination =
                Database::connect(&destination_url, database.installation_id().clone()).await?;
            destination.migrate().await?;
            destination.import_snapshot(&snapshot).await?;
            println!(
                "migrated installation {} from SQLite to PostgreSQL",
                database.installation_id()
            );
        }
    }
    Ok(())
}

fn write_snapshot(
    path: &PathBuf,
    snapshot: &crate::storage::DatabaseSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    serde_json::to_writer_pretty(BufWriter::new(file), snapshot)?;
    Ok(())
}

fn read_snapshot(
    path: &PathBuf,
) -> Result<crate::storage::DatabaseSnapshot, Box<dyn std::error::Error>> {
    Ok(serde_json::from_reader(BufReader::new(
        std::fs::File::open(path)?,
    ))?)
}

pub async fn execute_admin(
    database: &Database,
    command: AdminCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        AdminCommand::CreateUser {
            display_name,
            token,
            system_admin,
        } => {
            let principal = database
                .create_user(&display_name, &token, system_admin, unix_timestamp()?)
                .await?;
            println!(
                "created user {} ({})",
                principal.user_id, principal.display_name
            );
        }
        AdminCommand::GenerateEncryptionKey => {
            println!("{}", crate::crypto::EnvelopeCipher::generate_base64_key()?);
        }
    }
    Ok(())
}

fn unix_timestamp() -> Result<i64, std::time::SystemTimeError> {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(i64::try_from(seconds).unwrap_or(i64::MAX))
}
