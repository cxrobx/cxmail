use clap::Parser;
use cxmail_lib::tracker::{config, db as tracker_db, routes};
use cxmail_lib::LockExt;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    env_logger::init();

    let cli = config::Cli::parse();
    let command = cli.command.unwrap_or_default();

    match command {
        config::Commands::Serve {
            port,
            db_path,
            retention_days,
        } => {
            let conn = rusqlite::Connection::open(&db_path)
                .expect("Failed to open tracker database");
            tracker_db::initialize(&conn).expect("Failed to initialize tracker DB");

            let state = Arc::new(routes::AppState {
                db: std::sync::Mutex::new(conn),
            });

            // Spawn optional retention cleanup
            if retention_days > 0 {
                let state_clone = state.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                    loop {
                        interval.tick().await;
                        let conn = state_clone.db.safe_lock();
                        if let Ok(count) = tracker_db::cleanup_old_events(&conn, retention_days) {
                            if count > 0 {
                                log::info!("Cleaned up {} old open events", count);
                            }
                        }
                    }
                });
            }

            let app = routes::create_router(state);
            let addr = format!("0.0.0.0:{}", port);
            println!("cxmail-tracker listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            axum::serve(listener, app).await.unwrap();
        }
        config::Commands::GenerateKey { db_path, label } => {
            let conn = rusqlite::Connection::open(&db_path)
                .expect("Failed to open tracker database");
            tracker_db::initialize(&conn).expect("Failed to initialize tracker DB");

            // Generate a random API key
            let raw_key = uuid::Uuid::new_v4().to_string();
            let key_hash = tracker_db::hash_api_key(&raw_key);

            tracker_db::store_api_key_hash(&conn, &key_hash, label.as_deref())
                .expect("Failed to store API key");

            println!("API Key generated successfully!");
            println!();
            println!("  Key: {}", raw_key);
            println!();
            println!("Save this key — it cannot be retrieved later.");
            println!("Configure it in CXMail's tracking settings.");
        }
    }
}
