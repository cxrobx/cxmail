import ReactDOM from "react-dom/client";
import { useState } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { SignatureBlock, QuotedBlock, HtmlBlock } from "@/lib/composeNodes";
import { ComposerSelectionFeedback } from "@/components/shared/SelectionFeedback";
import MessageBody from "@/components/mail/MessageBody";
import { api } from "@/lib/tauri";
import { useWindowStore } from "@/stores/windowStore";
import { useChatStore } from "@/stores/chatStore";
import { useUIStore } from "@/stores/uiStore";
import type { MessageDetail } from "@/types/email";
import "../styles/globals.css";

// Browser-only harness: real editor/frame/bubble, with no mail or model access.
useUIStore.setState({ theme: "light" });
let delay = 30;
const rewrite = async () => { await new Promise((resolve) => setTimeout(resolve, delay)); return "Natural version"; };
Object.assign(api.ai, { adjustTone: rewrite, rewriteText: rewrite });
Object.assign(api.imageTrust, { isTrusted: async () => false });
Object.assign(api.chat, {
  start: async () => ({ gen: 1, cwd: "", repo: null, missing_repo_path: null, readable_repos: [], mcp_servers: [], seed_subject: "Test email" }),
  send: async () => {},
});

const email: MessageDetail = {
  uid: 42, subject: "Test email", from_name: "Test Sender", from_email: "sender@example.test",
  to_list: [{ name: null, email: "me@example.test" }], cc_list: [], bcc_list: [],
  date: "2026-10-13T12:00:00Z", plain_text: "Incoming plain text passage for feedback.",
  sanitized_html: "<p>Incoming email passage for feedback.</p>", attachments: [], is_read: true,
  is_flagged: false, list_unsubscribe: null, list_unsubscribe_post: null,
  message_id: "<email@example.test>", references: "<parent@example.test>", in_reply_to: null,
};

function Harness() {
  const [changed, setChanged] = useState(false);
  const editor = useEditor({
    extensions: [StarterKit, SignatureBlock, QuotedBlock, HtmlBlock],
    content: '<p>Draft passage for feedback.</p><div class="cx-html-block"><p>Designed block words.</p></div><div data-cx-signature="1" class="email-signature"><p>Signature words.</p></div><blockquote class="cx-quote" data-cx-quote="1"><p>Quoted history words.</p></blockquote>',
  });
  const windows = useWindowStore((state) => state.windows);
  const items = useChatStore((state) => state.items);
  return <main className="mx-auto max-w-2xl space-y-8 p-12 text-content">
    <h1 className="text-xl">Selection feedback browser check</h1>
    <section data-testid="composer" className="rounded-lg border border-border p-4">
      <h2>Draft</h2><EditorContent editor={editor} />
      {editor && <ComposerSelectionFeedback editor={editor} />}
    </section>
    <button onClick={() => { delay = 800; }}>Slow model</button>
    <button onClick={() => { editor?.commands.insertContentAt(1, "changed "); setChanged(true); }}>Change draft</button>
    <span>{changed ? "Draft changed" : "Draft unchanged"}</span>
    <section data-testid="email" className="rounded-lg border border-border p-4">
      <h2>Email</h2><MessageBody detail={email} accountId="email-account" folder="Archive" />
    </section>
    <section data-testid="plain"><MessageBody detail={{ ...email, uid: 43, sanitized_html: null }} accountId="plain-account" folder="INBOX" /></section>
    <pre data-testid="replies">{JSON.stringify(windows.map((window) => window.props))}</pre>
    <pre data-testid="chat">{JSON.stringify(items)}</pre>
    <button onClick={() => { editor?.commands.undo(); }}>Undo</button>
  </main>;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<Harness />);
