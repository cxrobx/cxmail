//! Text extraction from bounded, inert attachment bytes. No disk paths; a
//! malformed document never fails parsing the email around it.
//!
//! **PDF and DOCX are parsed in a separate, sandboxed process**
//! (`cxmail-extract`, src/bin/extract.rs): no network, no file writes, no
//! forking, no reads under the user's home folder, 5 s of CPU, 512 MB of
//! memory, and a wall-clock kill. The parsers are memory-safe Rust, so this is
//! not about the overrun bugs behind native-renderer PDF CVEs; it is about what
//! Rust does not stop: a loop or a decompression bomb in a crafted file would
//! otherwise stall a sync or an MCP call, or exhaust memory. Plain text stays
//! in-process.
//!
//! Release builds never fall back to in-process parsing of a complex format:
//! if the helper is missing, the attachment simply has no text. Debug builds
//! and tests (where the app package's bins are not built) parse in-process.
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_TEXT: usize = 100_000;
const HELPER: &str = "cxmail-extract";
const WALL_CLOCK: Duration = Duration::from_secs(10);

/// SBPL for the helper. `HOME` and `BIN` are substituted at spawn time. The
/// last matching rule wins, so the helper's own binary is re-allowed after the
/// blanket home-folder denial (a dev build lives under ~/Projects).
pub const SANDBOX_PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny file-write*)
(allow file-write-data (literal "/dev/null"))
(deny process-fork)
(deny file-read* (subpath "HOME"))
(allow file-read* (literal "BIN"))
"#;

fn is_complex(name: &str, mime: &str) -> bool {
    let n = name.to_ascii_lowercase();
    mime == "application/pdf"
        || n.ends_with(".pdf")
        || n.ends_with(".docx")
        || mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
}

fn helper_path() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join(HELPER);
    p.is_file().then_some(p)
}

/// Text from an attachment. PDF/DOCX go through the sandboxed helper.
pub fn extract(name: &str, mime: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    if !is_complex(name, mime) {
        return extract_in_process(name, mime, bytes);
    }
    match helper_path() {
        Some(bin) => extract_isolated(&bin, name, mime, bytes),
        None if cfg!(debug_assertions) => extract_in_process(name, mime, bytes),
        None => {
            log::warn!("{HELPER} not found next to {:?}; skipping text extraction", std::env::current_exe().ok());
            None
        }
    }
}

/// Run the helper under `sandbox-exec`, feed it the bytes, and give it
/// `WALL_CLOCK` to answer. Any failure is "no text", never an error that
/// could fail the email around the attachment.
pub fn extract_isolated(bin: &std::path::Path, name: &str, mime: &str, bytes: &[u8]) -> Option<String> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let profile = SANDBOX_PROFILE
        .replace("HOME", &home.to_string_lossy().replace('"', ""))
        .replace("BIN", &bin.to_string_lossy().replace('"', ""));
    let mut child = Command::new("/usr/bin/sandbox-exec")
        .arg("-p")
        .arg(profile)
        .arg(bin)
        .arg(name.replace('\0', ""))
        .arg(mime.replace('\0', ""))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stdout).take((MAX_TEXT * 4 + 1) as u64).read_to_end(&mut out);
        out
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() < WALL_CLOCK => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                log::warn!("{HELPER} timed out on {name:?}");
                break None;
            }
        }
    };
    let _ = writer.join();
    let out = reader.join().ok()?;
    if !status?.success() {
        return None;
    }
    let text: String = String::from_utf8_lossy(&out).chars().take(MAX_TEXT).collect();
    (!text.trim().is_empty()).then_some(text)
}

/// The parser itself. Only the helper binary and debug fallback call this for
/// complex formats.
pub fn extract_in_process(name: &str, mime: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let text = std::panic::catch_unwind(|| -> Option<String> {
        if mime == "application/pdf" || name.ends_with(".pdf") {
            pdf_extract::extract_text_from_mem(bytes).ok()
        } else if name.ends_with(".docx")
            || mime == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
            let file = zip.by_name("word/document.xml").ok()?;
            if file.size() > 2 * 1024 * 1024 {
                return None;
            }
            let mut xml = String::new();
            file.take(2 * 1024 * 1024 + 1)
                .read_to_string(&mut xml)
                .ok()?;
            if xml.len() > 2 * 1024 * 1024 {
                return None;
            }
            let doc = roxmltree::Document::parse(&xml).ok()?;
            let mut text = String::new();
            for n in doc.descendants() {
                if n.is_element() && n.tag_name().name() == "t" {
                    text.push_str(n.text().unwrap_or(""));
                }
                if n.is_element() && matches!(n.tag_name().name(), "p" | "br" | "tab") {
                    text.push('\n');
                }
                if text.len() > MAX_TEXT * 4 {
                    break;
                }
            }
            Some(text)
        } else if mime.starts_with("text/")
            || [".txt", ".md", ".csv", ".tsv", ".json"]
                .iter()
                .any(|e| name.ends_with(e))
        {
            let raw = std::str::from_utf8(bytes).ok()?;
            if mime == "text/html" || name.ends_with(".html") {
                Some(crate::email::parser::html_to_plain_text(raw))
            } else {
                Some(raw.to_string())
            }
        } else {
            None
        }
    })
    .ok()
    .flatten()?;
    let text: String = text.chars().take(MAX_TEXT).collect();
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod sandbox_tests {
    //! Prove the profile, not the parser: stand-in binaries try what a
    //! compromised extractor would. macOS only (sandbox-exec).
    use super::*;
    use std::path::Path;

    #[test]
    #[cfg(target_os = "macos")]
    fn the_sandbox_denies_reads_in_home_but_the_helper_protocol_works_elsewhere() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let inside = home.join(format!(".cxm-sandbox-probe-{}", std::process::id()));
        std::fs::write(&inside, "secret-in-home").unwrap();
        let outside = std::env::temp_dir().join(format!("cxm-sandbox-probe-{}", std::process::id()));
        std::fs::write(&outside, "fine-outside").unwrap();
        // `cat <name> <mime>` prints both files when allowed.
        let got_inside = extract_isolated(Path::new("/bin/cat"), inside.to_str().unwrap(), "/dev/null", b"");
        let got_outside = extract_isolated(Path::new("/bin/cat"), outside.to_str().unwrap(), "/dev/null", b"");
        let _ = std::fs::remove_file(&inside);
        let _ = std::fs::remove_file(&outside);
        assert!(got_inside.is_none(), "read inside HOME must be denied: {got_inside:?}");
        assert_eq!(got_outside.as_deref(), Some("fine-outside"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_sandbox_denies_writes() {
        let target = std::env::temp_dir().join(format!("cxm-sandbox-write-{}", std::process::id()));
        let _ = extract_isolated(Path::new("/usr/bin/touch"), target.to_str().unwrap(), "/dev/null", b"");
        assert!(!target.exists(), "a write from inside the sandbox must fail");
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "takes WALL_CLOCK (10 s); run with --ignored"]
    fn a_hung_extractor_is_killed_at_the_wall_clock() {
        let t = Instant::now();
        // `tail -f /dev/null` is silent and never exits, so only the wall clock
        // can end it. (`yes` is the wrong probe: the output cap closes the pipe
        // and SIGPIPE ends it in milliseconds, which is the cap working.)
        assert!(extract_isolated(Path::new("/usr/bin/tail"), "-f", "/dev/null", b"").is_none());
        let took = t.elapsed();
        assert!(took >= WALL_CLOCK && took < WALL_CLOCK + Duration::from_secs(3), "{took:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_docx_paragraph_text_and_ignores_markup() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Cancellation policy</w:t></w:r></w:p><w:p><w:r><w:t>Fourteen days notice</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        let data = zip.finish().unwrap().into_inner();
        let text = extract("policy.docx", "application/octet-stream", &data).unwrap();
        assert!(text.contains("Cancellation policy"));
        assert!(text.contains("Fourteen days notice"));
        assert!(!text.contains("w:body"));
    }
    #[test]
    fn reads_actual_pdf_text() {
        use std::io::Write;
        let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_string(),"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),{
      let stream="BT /F1 12 Tf 72 720 Td (Cancellation requires fourteen days notice.) Tj ET";format!("<< /Length {} >>\nstream\n{stream}\nendstream",stream.len())
   },"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string()];
        let mut data = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(data.len());
            writeln!(data, "{} 0 obj\n{object}\nendobj", i + 1).unwrap();
        }
        let xref = data.len();
        writeln!(data, "xref\n0 6\n0000000000 65535 f ").unwrap();
        for offset in offsets {
            writeln!(data, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            data,
            "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
        )
        .unwrap();
        let text = extract("policy.pdf", "application/pdf", &data).unwrap();
        assert!(text.contains("Cancellation requires fourteen days notice"));
    }
    #[test]
    fn refuses_binary_oversized_and_invalid_pdf() {
        assert!(extract("data.exe", "application/octet-stream", b"text").is_none());
        assert!(extract("a.pdf", "application/pdf", b"broken").is_none());
        assert!(extract("x.txt", "text/plain", &vec![b'a'; MAX_BYTES + 1]).is_none());
    }
    #[test]
    fn extracts_text_without_running_html() {
        assert_eq!(
            extract("x.txt", "text/plain", "résumé".as_bytes()).unwrap(),
            "résumé"
        );
        assert!(
            extract("x.html", "text/html", b"<p>Cancellation policy</p>")
                .unwrap()
                .contains("Cancellation")
        );
    }
}
