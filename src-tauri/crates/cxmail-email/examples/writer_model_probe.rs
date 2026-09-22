//! Set / get / clear the configured default writer model in the credential
//! store THIS process resolves — outside a .app bundle that is the file store
//! (credentials.dat), which is exactly what the loose MCP binary reads first.
//! E2E harness for the configured-default half of the writer-model precedence
//! (per-call `writer_model` > this setting > built-in default).
//!
//!   cargo run -p cxmail-email --example writer_model_probe -- get
//!   cargo run -p cxmail-email --example writer_model_probe -- set gemini-3.6-flash-low
//!   cargo run -p cxmail-email --example writer_model_probe -- clear

use cxmail_email::email::external_writer;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("get") | None => {
            println!(
                "configured: {:?}\neffective:  {}",
                external_writer::configured_model(),
                external_writer::effective_default_model()
            );
            Ok(())
        }
        Some("set") => match args.get(1) {
            Some(model) => external_writer::save_configured_model(model)
                .map(|()| println!("set {model}")),
            None => {
                eprintln!("usage: writer_model_probe set <model-id>");
                std::process::exit(2);
            }
        },
        Some("clear") => external_writer::save_configured_model("").map(|()| println!("cleared")),
        Some(other) => {
            eprintln!("unknown subcommand {other:?} — use get | set <id> | clear");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
