use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use uuid::Uuid;

/// Generate a unique, URL-safe pixel code (11 characters).
pub fn generate_pixel_code() -> String {
    let uuid = Uuid::new_v4();
    let bytes = &uuid.as_bytes()[..8];
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Inject a 1x1 tracking pixel into HTML email body.
/// Inserts before `</body>` if present, otherwise appends.
pub fn inject_tracking_pixel(html: &str, service_url: &str, pixel_code: &str) -> String {
    let pixel_tag = format!(
        r#"<img src="{}/t/{}.png" width="1" height="1" style="display:none" alt="" />"#,
        service_url.trim_end_matches('/'),
        pixel_code
    );
    if let Some(pos) = html.to_lowercase().rfind("</body>") {
        let mut result = html.to_string();
        result.insert_str(pos, &pixel_tag);
        result
    } else {
        format!("{}{}", html, pixel_tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_code_uniqueness() {
        let a = generate_pixel_code();
        let b = generate_pixel_code();
        assert_ne!(a, b);
        assert_eq!(a.len(), 11);
    }

    #[test]
    fn test_pixel_code_url_safe() {
        let code = generate_pixel_code();
        assert!(code.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_inject_with_body_tag() {
        let html = "<html><body><p>Hello</p></body></html>";
        let result = inject_tracking_pixel(html, "https://track.example.com", "abc123");
        assert!(result.contains(r#"<img src="https://track.example.com/t/abc123.png""#));
        assert!(result.contains(r#"</body></html>"#));
        // Pixel should be before </body>
        let pixel_pos = result.find("track.example.com").unwrap();
        let body_pos = result.rfind("</body>").unwrap();
        assert!(pixel_pos < body_pos);
    }

    #[test]
    fn test_inject_without_body_tag() {
        let html = "<p>Hello world</p>";
        let result = inject_tracking_pixel(html, "https://track.example.com", "abc123");
        assert!(result.starts_with("<p>Hello world</p>"));
        assert!(result.ends_with(r#"alt="" />"#));
    }

    #[test]
    fn test_inject_strips_trailing_slash() {
        let html = "<body></body>";
        let result = inject_tracking_pixel(html, "https://track.example.com/", "abc123");
        assert!(result.contains("https://track.example.com/t/abc123.png"));
        assert!(!result.contains("//t/"));
    }
}
