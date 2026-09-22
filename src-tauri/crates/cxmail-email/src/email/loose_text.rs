//! Give bare text inside a table cell an element of its own.
//!
//! ## The defect
//!
//! Designed email HTML (`layout: "card"` / `"rich"`) is hand-built by an agent,
//! and the shape it keeps producing is a styled cell whose paragraph copy is a
//! **bare text node** — a direct child of the `<td>`, wrapped in nothing:
//!
//! ```html
//! <td style="padding:14px 18px;font-size:15px;color:#334155">
//!   <div style="font-weight:bold">TL;DR</div>
//!   The growth is now a trend, not a hope.
//! </td>
//! ```
//!
//! Text with no element around it cannot be styled, which costs three things:
//!
//! 1. Every per-paragraph declaration the author wrote for it — `margin`,
//!    `font-size`, `color`, `line-height` — silently applies to nothing,
//!    because there is no element to carry it. The author sees their design
//!    ignored and nothing anywhere reports a problem.
//! 2. The source's own pretty-printing paints. The newline and indent before
//!    the text collapse to a single leading space, so the line renders with a
//!    first-line indent nobody wrote.
//! 3. Anything that later styles blocks by tag name — this project's
//!    [`crate::email::inline_styles::apply_inline_font_styles`], a mail client's
//!    own defaults, a `contenteditable` host normalizing on edit — has no hook
//!    for that text, so its rendering is decided by whatever the surrounding
//!    context happens to inherit rather than by the author.
//!
//! Wrapping the run in `<p style="margin:0">` makes the defect unrepresentable
//! rather than describing it in a tool description an agent may skip (the same
//! posture as `mcp::server::validate_pinned_rule`).
//!
//! ## Why the wrapper carries four `inherit` declarations
//!
//! `apply_inline_font_styles` runs on every outgoing draft *after* this pass and
//! prepends `font-family:Arial…;font-size:13px;color:#222222` to every `<p>`.
//! A wrapper of plain `<p style="margin:0">` would therefore **restyle** the
//! text this pass just wrapped: a 15px slate paragraph inside a designed card
//! would come out 13px `#222222`, and a `font-size:0` spacer cell would grow a
//! line box. That is a different silent defect, introduced by the fix.
//!
//! `font-family` / `font-size` / `line-height` / `color` set to `inherit` sit
//! *after* the injected base (CSS last-wins) and are not matched by
//! `inline_styles::OUR_DECL_RE`, so they survive the merge and the text keeps
//! exactly the typography it had as a bare node. The wrapper adds a styling
//! hook and removes stray whitespace; it changes nothing else. **Do not
//! "simplify" the wrapper down to `margin:0`** — `wrapper_survives_font_pass`
//! is the tripwire.
//!
//! ## What it will not do
//!
//! - **Never outside `<td>`/`<th>`.** Loose text in a `<div>` already forms an
//!   anonymous block box that inherits correctly and takes no default margin,
//!   so wrapping it would be churn with a real downside (one more element for
//!   the font pass to style). Loose text directly inside `<table>`/`<tr>` is a
//!   different bug — html5ever foster-parents it out of the table before this
//!   pass ever sees the result — and a `<p>` there would be foster-parented
//!   just the same, so this pass leaves it alone.
//! - **Never splits a line.** The unit wrapped is the whole *inline run*, text
//!   and inline elements together. Wrapping only the text nodes of
//!   `Hello <b>world</b> again` would turn one sentence into three blocks.
//! - **Never wraps a run with no bare text.** A cell holding only
//!   `<a>`/`<span>`/`<img>` is already styleable; leaving it byte-identical
//!   keeps this pass invisible on the designs that were correct all along.
//! - **Never wraps a spacer.** `<td style="height:4px">&nbsp;</td>` is the
//!   standard email spacer-bar idiom; `&nbsp;` runs count as whitespace here.
//! - **Never loses a byte.** Every token is re-emitted in source order; the only
//!   edits are the inserted wrapper tags and ASCII whitespace trimmed from the
//!   ends of a wrapped run (NBSP is deliberately *not* trimmed — it is content).

/// Opening tag of the wrapper. See the module docs: every declaration after
/// `margin:0` is load-bearing against `apply_inline_font_styles`.
pub const WRAPPER_OPEN: &str = concat!(
    r#"<p style="margin:0;font-family:inherit;font-size:inherit;"#,
    r#"line-height:inherit;color:inherit">"#
);

/// Closing tag of the wrapper.
pub const WRAPPER_CLOSE: &str = "</p>";

/// Outcome of a normalization pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LooseTextFix {
    pub html: String,
    /// How many bare-text runs were given a wrapper.
    pub wrapped: usize,
    /// True when at least one wrapped run shared its cell with a block-level
    /// child. That is the shape the author most likely got wrong on purpose:
    /// they wrote a block for one line and bare text for the next, so only one
    /// of the two carries their styling. Cells whose entire content is one text
    /// run are normalized too, but they are not worth a word to the caller.
    pub mixed_content: bool,
}

impl LooseTextFix {
    pub fn changed(&self) -> bool {
        self.wrapped > 0
    }
}

/// True if `html` holds bare text in a cell that also holds a block child.
/// Cheap enough to call for an advisory; it runs the full pass and throws the
/// output away.
pub fn has_mixed_cell_content(html: &str) -> bool {
    normalize_cell_text(html).mixed_content
}

/// Wrap every bare-text run inside a `<td>`/`<th>` in [`WRAPPER_OPEN`].
///
/// Idempotent: a run already inside a `<p>` (or any element) is not a bare text
/// node, so a second pass finds nothing to do.
pub fn normalize_cell_text(html: &str) -> LooseTextFix {
    let toks = tokenize(html);

    let mut stack: Vec<Frame> = vec![Frame::new("", false)];
    let mut wrapped = 0usize;
    let mut mixed = false;

    for tok in toks {
        match tok {
            Tok::Text(raw) => {
                let frame = stack.last_mut().expect("root frame");
                frame.push_text(raw);
            }
            Tok::Raw(raw) => {
                // Comments and doctypes are neither block nor prose: carry them
                // along inside whatever run they sit in, without letting them
                // trigger a wrap on their own.
                let frame = stack.last_mut().expect("root frame");
                frame.push_neutral(raw);
            }
            Tok::Start {
                name,
                raw,
                self_closing,
            } => {
                if self_closing || is_void(name) {
                    let frame = stack.last_mut().expect("root frame");
                    frame.push_child(name, raw);
                } else {
                    let mut f = Frame::new(name, is_cell(name));
                    f.buf.push_str(raw);
                    stack.push(f);
                }
            }
            Tok::End { name, raw } => {
                match stack
                    .iter()
                    .rposition(|f| f.name.eq_ignore_ascii_case(name))
                    .filter(|&idx| idx > 0)
                {
                    Some(idx) => {
                        // Anything still open above the match was left unclosed
                        // by the author; close it implicitly, exactly as a
                        // browser would, and without inventing an end tag.
                        while stack.len() - 1 > idx {
                            let f = stack.pop().expect("frame above match");
                            let (child, rendered) = f.finish(&mut wrapped, &mut mixed);
                            stack
                                .last_mut()
                                .expect("parent frame")
                                .push_child(&child, &rendered);
                        }
                        let f = stack.pop().expect("matched frame");
                        let (child, mut rendered) = f.finish(&mut wrapped, &mut mixed);
                        rendered.push_str(raw);
                        stack
                            .last_mut()
                            .expect("parent frame")
                            .push_child(&child, &rendered);
                    }
                    // A stray end tag closes nothing. Emit it where it stands.
                    None => stack.last_mut().expect("root frame").push_neutral(raw),
                }
            }
        }
    }

    while stack.len() > 1 {
        let f = stack.pop().expect("unclosed frame");
        let (child, rendered) = f.finish(&mut wrapped, &mut mixed);
        stack
            .last_mut()
            .expect("parent frame")
            .push_child(&child, &rendered);
    }

    let (_, html) = stack.pop().expect("root frame").finish(&mut wrapped, &mut mixed);
    LooseTextFix {
        html,
        wrapped,
        mixed_content: mixed,
    }
}

// ─── the frame machine ────────────────────────────────────────────────────
//
// One frame per open element. Non-cell frames are pure accumulators, so the
// output is the input's bytes in the input's order. A cell frame additionally
// buffers the *inline run* it is currently inside, and decides at each block
// boundary whether that run needs a wrapper.

struct Frame {
    name: String,
    is_cell: bool,
    buf: String,
    /// Inline run collected so far (cell frames only).
    pending: String,
    /// The pending run contains bare text that is more than whitespace.
    pending_text: bool,
    /// This cell has at least one block-level child.
    saw_block: bool,
    wrapped: usize,
}

impl Frame {
    fn new(name: &str, is_cell: bool) -> Self {
        Frame {
            name: name.to_string(),
            is_cell,
            buf: String::new(),
            pending: String::new(),
            pending_text: false,
            saw_block: false,
            wrapped: 0,
        }
    }

    /// A bare text run.
    fn push_text(&mut self, raw: &str) {
        if self.is_cell {
            self.pending.push_str(raw);
            if is_meaningful_text(raw) {
                self.pending_text = true;
            }
        } else {
            self.buf.push_str(raw);
        }
    }

    /// Markup that is neither prose nor a block boundary (comment, doctype,
    /// stray end tag): rides along in the current run without triggering a wrap.
    fn push_neutral(&mut self, raw: &str) {
        if self.is_cell {
            self.pending.push_str(raw);
        } else {
            self.buf.push_str(raw);
        }
    }

    /// A fully rendered child element (or a void tag).
    fn push_child(&mut self, name: &str, rendered: &str) {
        if self.is_cell {
            if is_block(name) {
                self.flush_pending();
                self.saw_block = true;
                self.buf.push_str(rendered);
            } else {
                self.pending.push_str(rendered);
            }
        } else {
            self.buf.push_str(rendered);
        }
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        if self.pending_text {
            // ASCII whitespace only: the newline+indent of a pretty-printed
            // source must not paint as a first-line indent, but an NBSP the
            // author typed is content.
            let trimmed = self
                .pending
                .trim_matches(|c: char| c.is_ascii_whitespace());
            self.buf.push_str(WRAPPER_OPEN);
            self.buf.push_str(trimmed);
            self.buf.push_str(WRAPPER_CLOSE);
            self.wrapped += 1;
        } else {
            self.buf.push_str(&self.pending);
        }
        self.pending.clear();
        self.pending_text = false;
    }

    /// Close the frame: returns `(tag name, rendered HTML)` and folds this
    /// frame's counters into the pass totals.
    fn finish(mut self, wrapped: &mut usize, mixed: &mut bool) -> (String, String) {
        self.flush_pending();
        if self.wrapped > 0 {
            *wrapped += self.wrapped;
            if self.saw_block {
                *mixed = true;
            }
        }
        (self.name, self.buf)
    }
}

// ─── classification ───────────────────────────────────────────────────────

fn is_cell(name: &str) -> bool {
    name.eq_ignore_ascii_case("td") || name.eq_ignore_ascii_case("th")
}

/// Elements that end an inline run. Everything not listed here is treated as
/// inline, which is the safe default: mistaking an inline element for a block
/// would split a sentence, while mistaking a block for inline only leaves a
/// wrapper unmade. `script`/`style`/`textarea` are listed so their contents can
/// never be swept into a wrapped paragraph.
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "iframe",
    "li",
    "main",
    "nav",
    "noscript",
    "ol",
    "p",
    "pre",
    "script",
    "section",
    "style",
    "table",
    "tbody",
    "td",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "ul",
];

fn is_block(name: &str) -> bool {
    BLOCK_TAGS.iter().any(|t| name.eq_ignore_ascii_case(t))
}

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void(name: &str) -> bool {
    VOID_TAGS.iter().any(|t| name.eq_ignore_ascii_case(t))
}

/// Elements whose content is not markup and must be skipped wholesale.
const RAW_TEXT_TAGS: &[&str] = &["script", "style", "textarea", "title"];

fn is_raw_text(name: &str) -> bool {
    RAW_TEXT_TAGS.iter().any(|t| name.eq_ignore_ascii_case(t))
}

/// True when a text run is more than whitespace. `&nbsp;` and its numeric
/// spellings count as whitespace so the ubiquitous `<td>&nbsp;</td>` spacer
/// cell is left alone. (A literal U+00A0 is already whitespace to `trim`.)
fn is_meaningful_text(raw: &str) -> bool {
    let mut s = raw.to_string();
    for entity in ["&nbsp;", "&#160;", "&#xa0;", "&#xA0;", "&#x00a0;", "&#x00A0;"] {
        if s.contains(entity) {
            s = s.replace(entity, " ");
        }
    }
    !s.trim().is_empty()
}

// ─── tokenizer ────────────────────────────────────────────────────────────
//
// Byte scanning is safe here because every byte it matches on is ASCII, and a
// UTF-8 continuation byte is never ASCII — so no slice can land mid-character
// (gotcha #21b: a length check is not a boundary check; an ASCII match is).

enum Tok<'a> {
    Text(&'a str),
    Start {
        name: &'a str,
        raw: &'a str,
        self_closing: bool,
    },
    End {
        name: &'a str,
        raw: &'a str,
    },
    /// Comment, doctype, processing instruction, or the contents of a raw-text
    /// element: copied through untouched.
    Raw(&'a str),
}

fn tokenize(input: &str) -> Vec<Tok<'_>> {
    let b = input.as_bytes();
    let mut toks: Vec<Tok> = Vec::new();
    let mut text_start = 0usize;
    let mut i = 0usize;

    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }

        let Some((end, tok)) = markup_at(input, i) else {
            // A `<` that starts no tag is literal text, exactly as a browser
            // treats it.
            i += 1;
            continue;
        };

        if text_start < i {
            toks.push(Tok::Text(&input[text_start..i]));
        }

        let raw_text_name = match &tok {
            Tok::Start {
                name, self_closing, ..
            } if !self_closing && is_raw_text(name) => Some(*name),
            _ => None,
        };
        toks.push(tok);
        i = end;
        text_start = end;

        // The contents of <script>/<style>/<textarea>/<title> are not markup.
        if let Some(name) = raw_text_name {
            let close = find_close_tag(input, i, name).unwrap_or(b.len());
            if close > i {
                toks.push(Tok::Raw(&input[i..close]));
            }
            i = close;
            text_start = close;
        }
    }

    if text_start < b.len() {
        toks.push(Tok::Text(&input[text_start..]));
    }
    toks
}

/// Parse the markup construct starting at `i` (which is a `<`). Returns the
/// index one past its `>` and the token, or `None` if this is a literal `<`.
fn markup_at(input: &str, i: usize) -> Option<(usize, Tok<'_>)> {
    let b = input.as_bytes();
    let next = *b.get(i + 1)?;

    if next == b'!' {
        if input[i..].starts_with("<!--") {
            let end = input[i + 4..]
                .find("-->")
                .map(|p| i + 4 + p + 3)
                .unwrap_or(b.len());
            return Some((end, Tok::Raw(&input[i..end])));
        }
        let end = find_byte(b, i + 2, b'>').map(|p| p + 1).unwrap_or(b.len());
        return Some((end, Tok::Raw(&input[i..end])));
    }

    if next == b'?' {
        let end = find_byte(b, i + 2, b'>').map(|p| p + 1).unwrap_or(b.len());
        return Some((end, Tok::Raw(&input[i..end])));
    }

    if next == b'/' {
        let name_start = i + 2;
        let name_end = name_end_from(b, name_start);
        if name_end == name_start {
            return None;
        }
        let end = find_byte(b, name_end, b'>').map(|p| p + 1).unwrap_or(b.len());
        return Some((
            end,
            Tok::End {
                name: &input[name_start..name_end],
                raw: &input[i..end],
            },
        ));
    }

    if !next.is_ascii_alphabetic() {
        return None;
    }

    let name_start = i + 1;
    let name_end = name_end_from(b, name_start);
    let (end, self_closing) = scan_start_tag(b, name_end);
    Some((
        end,
        Tok::Start {
            name: &input[name_start..name_end],
            raw: &input[i..end],
            self_closing,
        },
    ))
}

/// Walk a tag name: ASCII alphanumerics plus the few punctuation characters a
/// tag name may carry.
fn name_end_from(b: &[u8], start: usize) -> usize {
    let mut j = start;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b':') {
        j += 1;
    }
    j
}

/// Scan attributes to the tag's closing `>`, honoring quoted values (an
/// unescaped `>` inside `style="…"` must not end the tag). Returns the index
/// one past `>` and whether the tag was self-closed.
fn scan_start_tag(b: &[u8], from: usize) -> (usize, bool) {
    let mut j = from;
    let mut quote: Option<u8> = None;
    while j < b.len() {
        let c = b[j];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'>' => {
                    let mut k = j;
                    while k > from && b[k - 1].is_ascii_whitespace() {
                        k -= 1;
                    }
                    let self_closing = k > from && b[k - 1] == b'/';
                    return (j + 1, self_closing);
                }
                _ => {}
            },
        }
        j += 1;
    }
    (b.len(), false)
}

fn find_byte(b: &[u8], from: usize, needle: u8) -> Option<usize> {
    (from..b.len()).find(|&j| b[j] == needle)
}

/// Index of the `<` that opens `</name` at or after `from`, case-insensitive.
fn find_close_tag(input: &str, from: usize, name: &str) -> Option<usize> {
    let b = input.as_bytes();
    let mut j = from;
    while let Some(pos) = find_byte(b, j, b'<') {
        let after = pos + 1;
        if b.get(after) == Some(&b'/') {
            let ns = after + 1;
            let ne = name_end_from(b, ns);
            if input[ns..ne].eq_ignore_ascii_case(name) {
                return Some(pos);
            }
        }
        j = pos + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real 2026-08-18 shape: an eyebrow block, then the paragraph copy as
    /// a bare text node, pretty-printed onto its own indented line.
    const XRF_CELL: &str = "<table><tbody><tr>\
<td style=\"background-color:#f0f5f1;padding:14px 18px;\">\n\
            <div style=\"font-weight:bold;margin-bottom:6px\">TL;DR</div>\n\
            The growth is now a trend, not a hope.\n\
          </td></tr></tbody></table>";

    #[test]
    fn wraps_bare_text_beside_a_block_in_a_cell() {
        let fix = normalize_cell_text(XRF_CELL);
        assert_eq!(fix.wrapped, 1, "expected one wrapped run: {}", fix.html);
        assert!(fix.mixed_content, "block sibling should be reported");
        assert!(
            fix.html
                .contains(&format!("{WRAPPER_OPEN}The growth is now a trend, not a hope.</p>")),
            "run not wrapped tightly: {}",
            fix.html
        );
        // The eyebrow was already an element — untouched.
        assert!(
            fix.html
                .contains("<div style=\"font-weight:bold;margin-bottom:6px\">TL;DR</div>"),
            "existing block disturbed: {}",
            fix.html
        );
    }

    #[test]
    fn source_indentation_does_not_survive_as_an_indent() {
        // The second half of the visible defect: a pretty-printed newline and
        // indent collapse to a leading space in the rendered line. The wrapped
        // run must start at its first real character.
        let fix = normalize_cell_text(XRF_CELL);
        let open = fix.html.find(WRAPPER_OPEN).expect("wrapper present");
        let body = &fix.html[open + WRAPPER_OPEN.len()..];
        assert!(
            body.starts_with("The growth"),
            "leading whitespace survived: {body:?}"
        );
        assert!(
            !fix.html.contains(&format!(" {WRAPPER_CLOSE}")),
            "trailing whitespace survived: {}",
            fix.html
        );
    }

    #[test]
    fn is_idempotent() {
        let once = normalize_cell_text(XRF_CELL);
        let twice = normalize_cell_text(&once.html);
        assert_eq!(twice.wrapped, 0, "second pass wrapped again: {}", twice.html);
        assert_eq!(once.html, twice.html);
    }

    #[test]
    fn already_wrapped_text_is_returned_byte_for_byte() {
        // The correctly-authored version of the same cell, and the guarantee
        // that matters for every designed email that was already right.
        for html in [
            r#"<table><tr><td style="padding:14px"><p style="margin:0">Copy.</p></td></tr></table>"#,
            r#"<td><div>a</div><div>b</div></td>"#,
            r#"<td><a href="https://x.test" style="display:inline-block">Book a time</a></td>"#,
            r#"<td><ul><li>one</li><li>two</li></ul></td>"#,
            r#"<p>Ordinary prose with no table at all.</p>"#,
            r#"<div style="padding:20px">Loose text in a div stays loose.</div>"#,
        ] {
            let fix = normalize_cell_text(html);
            assert_eq!(fix.wrapped, 0, "unexpected wrap in {html}");
            assert_eq!(fix.html, html, "input mutated: {html}");
        }
    }

    #[test]
    fn spacer_cells_are_left_alone() {
        // The standard 4px rule-bar idiom. Wrapping it would hand the font pass
        // a <p> to style and the bar would grow a line box.
        for html in [
            r#"<td style="height:4px;font-size:0;line-height:0;">&nbsp;</td>"#,
            r#"<td>&#160;</td>"#,
            "<td>\n   \n</td>",
        ] {
            let fix = normalize_cell_text(html);
            assert_eq!(fix.wrapped, 0, "spacer wrapped: {html}");
            assert_eq!(fix.html, html);
        }
    }

    #[test]
    fn wraps_the_whole_inline_run_not_each_text_node() {
        // Wrapping text nodes individually would turn one sentence into three
        // blocks and drop the spaces around the bold word.
        let fix = normalize_cell_text("<td>Hello <b>world</b> again</td>");
        assert_eq!(fix.wrapped, 1);
        assert_eq!(
            fix.html,
            format!("<td>{WRAPPER_OPEN}Hello <b>world</b> again{WRAPPER_CLOSE}</td>")
        );
    }

    #[test]
    fn a_run_of_only_inline_elements_is_not_wrapped() {
        // No bare text means nothing is unstyleable; leaving it alone keeps this
        // pass invisible on designs that were already correct.
        let fix = normalize_cell_text(r#"<td><span style="color:red">x</span><img src="cid:a"></td>"#);
        assert_eq!(fix.wrapped, 0);
    }

    #[test]
    fn each_run_between_blocks_gets_its_own_wrapper() {
        let fix = normalize_cell_text("<td>lead<div>block</div>tail</td>");
        assert_eq!(fix.wrapped, 2);
        assert_eq!(
            fix.html,
            format!("<td>{WRAPPER_OPEN}lead{WRAPPER_CLOSE}<div>block</div>{WRAPPER_OPEN}tail{WRAPPER_CLOSE}</td>")
        );
    }

    #[test]
    fn nested_cells_are_normalized_too() {
        // Designed email is tables inside tables; the outer cell's own text and
        // the inner cell's text are separate runs.
        let fix = normalize_cell_text(
            "<td>outer<table><tr><td>inner</td></tr></table></td>",
        );
        assert_eq!(fix.wrapped, 2, "{}", fix.html);
        assert!(fix.html.contains(&format!("{WRAPPER_OPEN}inner{WRAPPER_CLOSE}")));
        assert!(fix.html.contains(&format!("{WRAPPER_OPEN}outer{WRAPPER_CLOSE}")));
    }

    #[test]
    fn text_inside_an_element_in_a_cell_is_not_bare() {
        let html = r#"<td><p style="margin:0;font-size:14px">Styled.</p></td>"#;
        assert_eq!(normalize_cell_text(html).html, html);
    }

    #[test]
    fn mixed_content_flag_only_fires_beside_a_block() {
        // A text-only cell is normalized but is not worth telling the caller
        // about: there was no competing block whose styling it missed.
        assert!(!normalize_cell_text("<td>just text</td>").mixed_content);
        assert!(normalize_cell_text("<td>text<div>block</div></td>").mixed_content);
        // Order does not matter — the block may come after the run.
        assert!(normalize_cell_text("<td><div>block</div>text</td>").mixed_content);
        assert!(has_mixed_cell_content("<td>text<hr></td>"));
    }

    #[test]
    fn tag_interiors_are_never_treated_as_text() {
        // A `>` inside an attribute value must not end the tag, or the rest of
        // the style would be emitted as prose and wrapped.
        let html = r#"<td><div style="font-family:'a>b';color:red">x</div></td>"#;
        assert_eq!(normalize_cell_text(html).html, html);
    }

    #[test]
    fn raw_text_elements_are_copied_through() {
        let html = "<td><style>td > p { color: red }</style>copy</td>";
        let fix = normalize_cell_text(html);
        assert!(
            fix.html.contains("<style>td > p { color: red }</style>"),
            "style body mangled: {}",
            fix.html
        );
        assert_eq!(fix.wrapped, 1, "the prose beside it still wraps");
        assert!(!fix.html.contains(&format!("{WRAPPER_OPEN}<style>")));
    }

    #[test]
    fn comments_ride_along_without_forcing_a_wrap() {
        let fix = normalize_cell_text("<td><!-- note -->text</td>");
        assert_eq!(fix.wrapped, 1);
        assert!(fix.html.contains("<!-- note -->"), "{}", fix.html);
        assert_eq!(normalize_cell_text("<td><!-- only a note --></td>").wrapped, 0);
    }

    #[test]
    fn unclosed_and_stray_tags_do_not_lose_content() {
        // Malformed input must degrade to "changed nothing important", never to
        // dropped copy.
        for html in [
            "<td>text",
            "<td><div>open<td>second</td>",
            "<td>text</span></td>",
            "<td",
            "a < b and c > d",
        ] {
            let fix = normalize_cell_text(html);
            let stripped = fix.html.replace(WRAPPER_OPEN, "").replace(WRAPPER_CLOSE, "");
            assert_eq!(
                stripped.split_ascii_whitespace().collect::<Vec<_>>(),
                html.split_ascii_whitespace().collect::<Vec<_>>(),
                "content changed for {html:?}: {}",
                fix.html
            );
        }
    }

    #[test]
    fn multibyte_text_is_not_split() {
        let html = "<td>Café — naïve … 🙂</td>";
        let fix = normalize_cell_text(html);
        assert!(fix.html.contains("Café — naïve … 🙂"), "{}", fix.html);
    }

    #[test]
    fn uppercase_tags_are_recognized() {
        let fix = normalize_cell_text("<TD>text<DIV>block</DIV></TD>");
        assert_eq!(fix.wrapped, 1);
        assert!(fix.mixed_content);
    }

    #[test]
    fn wrapper_survives_font_pass_without_restyling_the_text() {
        // THE tripwire for the `inherit` declarations. `apply_inline_font_styles`
        // prepends its base to every <p>; without the inherit list the wrapped
        // text would silently become 13px #222222 inside a designed card.
        use crate::email::inline_styles::apply_inline_font_styles;

        let fix = normalize_cell_text(
            r#"<td style="font-size:15px;color:#334155">Designed copy.</td>"#,
        );
        let styled = apply_inline_font_styles(&fix.html);

        for decl in [
            "font-size:inherit",
            "color:inherit",
            "font-family:inherit",
            "line-height:inherit",
            "margin:0",
        ] {
            assert!(styled.contains(decl), "{decl} lost in font pass: {styled}");
        }
        // And the inherit declarations must come AFTER the injected base, or
        // last-wins hands the paragraph back to the base font.
        let base = styled.find("font-size:13px").expect("base injected");
        let inherit = styled.find("font-size:inherit").expect("inherit kept");
        assert!(
            base < inherit,
            "inherit must win by coming last: {styled}"
        );
    }
}
