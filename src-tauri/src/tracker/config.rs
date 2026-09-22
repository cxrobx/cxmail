use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cxmail-tracker", about = "Email open tracking service for CXMail")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the tracking server (default)
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "8090")]
        port: u16,

        /// Path to SQLite database
        #[arg(short, long, default_value = "tracker.db")]
        db_path: String,

        /// Auto-delete open events older than this many days (0 = no retention limit)
        #[arg(long, default_value = "0")]
        retention_days: u32,
    },
    /// Generate a new API key
    GenerateKey {
        /// Path to SQLite database
        #[arg(short, long, default_value = "tracker.db")]
        db_path: String,

        /// Label for the key
        #[arg(short, long)]
        label: Option<String>,
    },
}

impl Default for Commands {
    fn default() -> Self {
        Commands::Serve {
            port: 8090,
            db_path: "tracker.db".to_string(),
            retention_days: 0,
        }
    }
}
