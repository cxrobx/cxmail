//! Text extraction from bounded, inert attachment bytes. No disk paths or
//! processes; a malformed document never fails parsing the email around it.
use std::io::{Cursor, Read};

pub const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_TEXT: usize = 100_000;

pub fn extract(name: &str, mime: &str, bytes: &[u8]) -> Option<String> {
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
