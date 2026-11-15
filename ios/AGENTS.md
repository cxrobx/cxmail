# Native CXMail companion

Read `README.md`, `.claude/rules/design-system.md` and `../remote/README.md` first.
`project.yml` is the source of the generated Xcode project. Local endpoint/team
configuration is ignored; no provisioning key or machine hostname belongs in git.

The mini remains the only mail engine. Never add phone-side IMAP credentials,
expose the MCP directly, bypass Tailscale identity, or weaken certificate checks.
Preserve the reviewed-draft digest, full recipient review and explicit final
send confirmation. Never automatically retry a mutation after an ambiguous error.
Send review uses a compact floating glass capsule, never a full-width send CTA.
Scheduling must preserve the reviewed digest and Bcc review, require a separate
final approval and persist on the mini. Keep cancellation and uncertain delivery
outcomes visible; a pending schedule blocks immediate send of that same draft.
Only a typed `410 / draft_gone` response may evict a missing draft revision from
phone caches. Preserve unrelated drafts, local composer recovery and a newer
selection made during refresh. Connection failures and digest mismatches retain
the draft; never guess a replacement by subject or transfer send approval to it.

Before a phone install, run `scripts/verify.sh` on compact Duo + wide iPad
simulators. Fixtures are fiction-only and in-process; real mail must never be
mutated by tests. Assert rendered HTML, not just that the reader screen exists.
Keep primary actions in the safe content area: the Duo's side-rail hit testing
can misroute taps to the inbox underneath (caught by chat-open UI regression).

Keep mail-row hit targets rectangular, including blank label space. A pending
mark-read acknowledgement must not disable opening a row or scrolling its reader;
keep mutation guards on the actions. XCUITest row bounds can include the Duo’s
system rail: tap within usable content, and use its native `BackButton` identifier.

Automatic images use the mini's shared `image_policy`: exact sender trust,
current From-domain authentication and no suspicious/Junk flag are all required.
Never use cached `imagesTrusted` or a remembered positive policy as network
permission offline. Keep Show once separate from trust; preserve sandbox/CSP
restrictions when images are allowed. The sender indicator is domain
authentication, never Google's BIMI brand badge or a claim of content safety.

Fixed-width HTML must fit the actual WKWebView width and retain its full height
after rotation or pane resizing. Keep responsive mail at natural scale. Native
fitting uses constant app-authored code in `.defaultClient` with
`allowsContentJavaScript = false`; never interpolate mail into executable code
or enable page scripts to measure it. Preserve the fixed-table right-edge/footer
UI regression and native WebKit geometry/security tests.

MCP text output is not a message-body API: `read_email` includes diagnostic
headers, and its plain-text branch can be empty for HTML-only mail. The remote
API resolves missing bodies through the sanitized preview, verifies/removes its
envelope (including matching-nonce MCP safety fences and server-note metadata),
and returns `body_error` for a failed fetch. Keep the uncached HTML,
plain-only, empty-body, failure/retry and missing-snippet density regressions.

The exact-width test is a geometry harness, not physical hinge automation.
Include the asymmetric external safe area in open-window fixtures. Compare
rendered WKWebView bounds for pane spacing: a native split navigator's background
and scroll view can span both columns while its content is correctly inset.
Apply column-width preferences after toolbar modifiers; otherwise the native
sidebar can fall back to 320pt. Assert the actual 455.5pt inbox boundary and
40pt division, as well as the 400pt portrait drawer and its single close control.
When changing layout, preserve the root-owned selection, draft and chat state,
check both appearances and XXXL type, and inspect the test captures/recordings.

Chat input and poll ownership live in the shared ChatStore, never the presented
view. Docking must preserve the conversation, pending permissions and unsent text.
Dismissal while moving into split chat must not pause its poll. Gate docking by
usable width, not keyboard-reduced height. Test typing while docked, dragging the
divider and popping back out, then assert the original mail toolbar and width.
Context changes affect the next turn, not provider lifetime. Keep the exact
message reference in the transport prompt, the user's original text in the UI,
and history/draft intact. Never silently widen missing email context to all mail.
