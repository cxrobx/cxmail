// Live autodiscovery probe. Safe: performs no authentication and sends no
// credential — it only resolves settings. `cargo run --example discover -- a@b.com`
#[tokio::main]
async fn main() {
    env_logger::init();
    for email in std::env::args().skip(1) {
        match cxmail_lib::email::autoconfig::discover(&email).await {
            Some(c) => println!(
                "{:<28} [{}] imap {}:{} {} | smtp {}:{} {} | user {:?}",
                email, c.source, c.imap_host, c.imap_port, c.imap_security,
                c.smtp_host, c.smtp_port, c.smtp_security, c.imap_username
            ),
            None => println!("{:<28} [none]  no settings found", email),
        }
    }
}
