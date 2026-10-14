import ReactDOM from "react-dom/client";
import EmailFrame from "@/components/mail/EmailFrame";
import MessageListItem from "@/components/mail/MessageListItem";
import { api } from "@/lib/tauri";
import { useUIStore } from "@/stores/uiStore";
import "../styles/globals.css";

// Browser-only regression fixture. No account, mail or remote image access.
const html = `
<div style="background:#f0f0f0;padding:20px">
  <div id="white-card" style="background:white;border-radius:16px;padding:24px">
    <h1 id="receipt-name" style="color:#222">Alex Example</h1>
    <p id="receipt-date" style="color:#666">Today</p>
    <h1 id="receipt-amount" style="color:#222;font-size:40px">$50.00</h1>
    <hr style="border:1px solid #ddd">
    <h2 id="receipt-heading" style="color:#222">Transaction details</h2>
    <p id="receipt-status" style="color:#666">Payment sent successfully</p>
    <p id="inherited-white">Text inheriting from the white card</p>
    <p id="pale-on-white" style="color:#e8dcc8!important">Pale authored text on white</p>
    <a id="white-link" href="https://example.test/receipt">View receipt</a>
    <a id="blue-button" href="https://example.test/receipt" style="display:inline-block;background:navy;color:white;padding:6px">
      <span id="button-label">Read payment details</span>
    </a>
    <blockquote><p id="white-quote">Quoted text on white</p></blockquote>
    <div style="background:rgba(255,255,255,.5)"><span id="transparent-white">A transparent nested surface</span></div>
    <div style="color:rgba(0,0,0,.65)"><div style="background:#222"><span id="alpha-ink-on-dark">Inherited translucent ink on dark</span></div></div>
    <table bgcolor="#ffffff" style="width:100%"><tbody><tr><td id="legacy-white">Legacy table background</td></tr></tbody></table>
    <div style="background:hsl(210,30%,94%)"><font id="legacy-font" color="#222222">Legacy font color</font></div>
    <p id="inline-inherit" style="color:inherit">Explicitly inherited color</p>
    <div id="currentcolor-background" style="color:white;background:currentColor"><span id="currentcolor-text">Text on a currentColor surface</span></div>
    <span data-testid="hidden-preheader" style="color:transparent">Intentionally invisible preheader</span>
  </div>
</div>
<div id="dark-card" style="background:#222;padding:16px;color:#fff">
  <strong id="dark-card-text">Original white text on a dark card</strong>
  <div style="background:rgba(255,255,255,.85)"><p id="translucent-card-text">Light surface over a dark card</p></div>
</div>
<p id="plain-dark" style="color:black">Black text on the app's dark background</p>
<p id="plain-inherited">Unstyled correspondence</p>
<pre id="code-dark" style="color:#222">Preformatted mail stays readable too.</pre>
<img id="blocked-image" src="" data-original-src="https://example.test/tracker.png" alt="Blocked image">
`;

useUIStore.setState({ theme: "dark" });
Object.assign(api.imageTrust, { isTrusted: async () => false });
Object.assign(api.messages, { fetchCachedBody: async () => ({ sanitized_html: html, plain_text: "" }) });

function Harness() {
  const theme = useUIStore((state) => state.theme);
  const setTheme = (next: "dark" | "light") => {
    document.documentElement.setAttribute("data-theme", next);
    useUIStore.setState({ theme: next });
  };
  return <main style={{ background: theme === "dark" ? "#1c1a17" : "#faf7f2", padding: 24 }}>
    <nav className="mb-4 flex gap-4 text-content">
      <button onClick={() => setTheme("dark")}>Dark mode</button>
      <button onClick={() => setTheme("light")}>Light mode</button>
    </nav>
    <div style={{ maxWidth: 900, margin: "0 auto" }}>
      <div className="mb-4" data-testid="preview-row">
        <MessageListItem message={{
          uid: 1, account_id: "fixture", subject: "Payment sent — hover preview", from_name: "Example Sender", from_email: null,
          date: "2026-10-13T12:00:00Z", snippet: "A receipt with nested light and dark surfaces", is_read: true,
          is_flagged: false, has_attachments: false, size_bytes: 0, category: null, is_muted: false, is_pinned: false,
          thread_count: 0, thread_draft_count: 0, thread_root_id: null, thread_has_unread: false,
        }} isSelected={false} onClick={() => {}} />
      </div>
      <EmailFrame html={html} senderEmail="sender@example.test" />
    </div>
  </main>;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<Harness />);
