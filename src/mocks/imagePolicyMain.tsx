import { useState } from "react";
import ReactDOM from "react-dom/client";
import EmailFrame from "@/components/mail/EmailFrame";
import SenderAuthentication from "@/components/mail/SenderAuthentication";
import { api } from "@/lib/tauri";
import { IMAGE_TRUST_CHANGED } from "@/hooks/useMessageImagePolicy";
import type { ImagePolicy } from "@/types/email";
import { useUIStore } from "@/stores/uiStore";
import "../styles/globals.css";

// Fiction-only UI harness. No real IPC, mailbox access or image hosts.
let trusted = false;
let failed = false;
Object.assign(api.imageTrust, {
  policy: async (): Promise<ImagePolicy> => ({ authentication: { status: failed ? "failed" : "passed", domain: "example.org", provider: "Google", explanation: "Google authenticated the From domain. This does not verify the person, brand, links or attachments." }, sender_email: "news@example.org", sender_trusted: trusted, auto_load: trusted && !failed, suspicious: failed, has_remote_images: true, blocked_reason: failed ? "Domain authentication failed" : trusted ? null : "This sender is not trusted yet" }),
  trust: async () => { trusted = true; },
});
Object.assign(api.system, { logClientError: async () => {} });
useUIStore.setState({ theme: "light" });
const html = `<div style="background:white;padding:24px;color:#222"><h1>Weekly studio notes</h1><p>Hello Chris,</p><p>Here are this week's highlights.</p><img id="remote-photo" src="" data-original-src="https://images.example.org/photo.png" alt="Weekly illustration" width="240" height="100"><div id="remote-background" style="height:30px;background-image:url(https://images.example.org/background.png)">Image background</div><p><a href="https://example.org/weekly">Read this week's notes</a></p></div>`;
function Preview() {
  const [uid, setUid] = useState(10);
  return <main className="mx-auto max-w-3xl bg-surface p-6 text-content">
    <h1 className="text-xl font-semibold">Weekly studio notes</h1>
    <p className="my-3">From: Studio Notes &lt;news@example.org&gt; <SenderAuthentication accountId="fixture" folder="INBOX" uid={uid} /></p>
    <div className="mb-4 flex gap-4 text-sm">
      <button onClick={() => { failed = !failed; window.dispatchEvent(new Event(IMAGE_TRUST_CHANGED)); }}>Toggle authentication</button>
      <button onClick={() => { trusted = false; window.dispatchEvent(new Event(IMAGE_TRUST_CHANGED)); }}>Revoke sender trust</button>
      <button onClick={() => setUid(uid + 1)}>Next message</button>
    </div>
    <EmailFrame html={html} senderEmail="news@example.org" messageRef={{ accountId: "fixture", folder: "INBOX", uid }} />
  </main>;
}
ReactDOM.createRoot(document.getElementById("root")!).render(<Preview />);
