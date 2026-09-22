/**
 * Throwaway harness for driving the REAL AIAssistPanel in a real browser.
 *
 * jsdom cannot model focus, selection, drag or portals faithfully (gotcha
 * #33), and the panel's whole job is editing a live ProseMirror document. So
 * this mounts the actual component against an actual TipTap editor with IPC
 * stubbed, and Playwright drives it. Not shipped — `mock.html` only.
 */
import { useState } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { SignatureBlock, QuotedBlock, HtmlBlock } from "@/lib/composeNodes";
import { api } from "@/lib/tauri";
import AIAssistPanel, { type AssistLaunch } from "@/components/mail/AIAssistPanel";
import type { DraftSnapshot } from "@/lib/draftValidation";
import type { AiDraftFinding } from "@/types/email";

/* ── IPC stubs ──────────────────────────────────────────────── */

const AI_FINDINGS: AiDraftFinding[] = [
  {
    category: "pinned_rule",
    severity: "error",
    where: "Greeting",
    title: "Greeting breaks a pinned rule",
    detail:
      'Pinned rule for sam@harborline.example: always address as "Bro. Ellis". This draft opens "Hi Sam,".',
    quote: "Hi Sam,",
    suggestion: "Bro. Ellis,",
  },
  {
    category: "unanswered_question",
    severity: "warning",
    where: "Thread",
    title: "Their question is unanswered",
    detail: "Sam asked when the hosting fee starts billing; the draft never gives a date.",
    quote: null,
    suggestion: "Billing starts on the 1st of next month, so nothing hits before then.",
  },
  {
    category: "next_step",
    severity: "info",
    where: "Closing",
    title: "No clear next step",
    detail: "The message ends on information rather than an ask.",
    quote: null,
    suggestion: "Let me know if the 1st works and I'll get it queued.",
  },
];

let aiDelayMs = 400;

// `api` is a plain object literal, so its methods can be swapped in place.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const anyApi = api as any;
anyApi.ai.validateDraft = async () => {
  await new Promise((r) => setTimeout(r, aiDelayMs));
  return AI_FINDINGS;
};
anyApi.ai.rewriteText = async (text: string, instruction: string) => {
  await new Promise((r) => setTimeout(r, 250));
  return `[${instruction}]\n\n${text.slice(0, 160)}`;
};
anyApi.ai.generateCompose = async () => {
  await new Promise((r) => setTimeout(r, 250));
  return "Bro. Ellis —\n\nBoth of Skyla's CRM asks are live.";
};
anyApi.ai.generateReply = async () => {
  await new Promise((r) => setTimeout(r, 250));
  return "Bro. Ellis —\n\nThanks for the quick turnaround on this.";
};
anyApi.system.logClientError = async () => {};

/* ── Harness ────────────────────────────────────────────────── */

const START = `
<p>Hi Sam,</p>
<p>Updates first: both of Skyla's CRM asks are live, along with a couple of other improvements.</p>
<p>Separately, following up on the hosting fee. I've attached the invoice breakdown.</p>
<p>It comes to $20 a month, flat. Send it to [Name] once you have a moment.</p>
<p><a href="#">Here is the link to set it up.</a></p>
<div data-cx-signature="1" class="email-signature"><p>— Chris</p></div>
`;

export default function AssistHarness() {
  const [subject, setSubject] = useState("Re: CRM Updates Live and Hosting Fee Setup");
  const [launch, setLaunch] = useState<AssistLaunch | null>(null);
  const [selection, setSelection] = useState<{ from: number; to: number; text: string } | null>(
    null,
  );
  const [count, setCount] = useState<number | undefined>();

  const editor = useEditor({
    extensions: [StarterKit, SignatureBlock, QuotedBlock, HtmlBlock],
    content: START,
    editorProps: {
      attributes: { class: "prose prose-sm max-w-none focus:outline-none px-4 py-3" },
    },
  });

  const getDraft = (): DraftSnapshot => ({
    subject,
    to: [{ name: "Sam Ellis", email: "sam@harborline.example" }],
    cc: [],
    bcc: [],
    bodyText: editor?.getText() ?? "",
    bodyHtml: editor?.getHTML() ?? "",
    attachmentCount: 0,
    inReplyTo: null,
  });

  const open = (l: AssistLaunch) => {
    const sel = editor?.state.selection;
    setSelection(
      sel && sel.from !== sel.to
        ? { from: sel.from, to: sel.to, text: editor!.state.doc.textBetween(sel.from, sel.to, " ") }
        : null,
    );
    setLaunch(l);
  };

  return (
    <div className="min-h-screen bg-sidebar p-8 font-sans">
      <div className="mx-auto mb-4 flex max-w-[900px] flex-wrap items-center gap-2">
        <button
          data-testid="open-review"
          onClick={() => open({ mode: "review" })}
          className="rounded-lg bg-accent px-3 py-2 text-sm font-medium text-white"
        >
          Validate
        </button>
        <button
          data-testid="open-chat"
          onClick={() =>
            open({ mode: "chat", chat: { kind: "rewrite", instruction: "", autoRun: false } })
          }
          className="rounded-lg border border-border bg-base px-3 py-2 text-sm text-content-secondary"
        >
          Rewrite
        </button>
        <button
          data-testid="no-ai"
          onClick={() => {
            aiDelayMs = 100;
            anyApi.ai.validateDraft = async () => {
              throw new Error("AI provider not configured");
            };
          }}
          className="rounded-lg border border-border bg-base px-3 py-2 text-xs text-content-muted"
        >
          Make AI fail
        </button>
        <button
          data-testid="clean-draft"
          onClick={() => {
            editor?.commands.setContent(
              "<p>Bro. Ellis,</p><p>Both asks are live. Let me know if the 1st works.</p>",
            );
            setSubject("CRM updates are live");
          }}
          className="rounded-lg border border-border bg-base px-3 py-2 text-xs text-content-muted"
        >
          Load clean draft
        </button>
        <span data-testid="badge" className="text-xs text-content-muted">
          badge: {count === undefined ? "—" : count}
        </span>
      </div>

      <div className="mx-auto flex max-w-[900px] flex-col overflow-hidden rounded-xl border border-border bg-base shadow-macos-lg">
        <input
          data-testid="subject"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
          className="border-b border-border-subtle bg-transparent px-4 py-2 text-sm text-content outline-none"
        />
        <div data-testid="editor" className="min-h-[320px]">
          <EditorContent editor={editor} />
        </div>
      </div>

      {editor && launch && (
        <AIAssistPanel
          open
          launch={launch}
          onClose={() => setLaunch(null)}
          editor={editor}
          getDraft={getDraft}
          accountId="acct-1"
          recipientEmail="sam@harborline.example"
          subject={subject}
          onSubjectChange={setSubject}
          selection={selection}
          onReviewCount={setCount}
        />
      )}
    </div>
  );
}
