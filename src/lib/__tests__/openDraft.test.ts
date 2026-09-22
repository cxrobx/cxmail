import { describe, it, expect, beforeEach, vi } from "vitest";
import type { AttachmentMeta, MessageDetail } from "@/types/email";

vi.mock("@/lib/tauri", () => ({
  api: {
    messages: { fetchBody: vi.fn() },
    compose: { fetchOutgoingAttachments: vi.fn() },
    system: { logClientError: vi.fn(() => Promise.resolve()) },
  },
}));

import { api } from "@/lib/tauri";
import { openDraftForEdit } from "@/lib/draftCompose";
import { draftWindowKey, useWindowStore } from "@/stores/windowStore";

const fetchBody = vi.mocked(api.messages.fetchBody);
const fetchAttachments = vi.mocked(api.compose.fetchOutgoingAttachments);
const logClientError = vi.mocked(api.system.logClientError);

const A = "acct-cxv";
const windows = () => useWindowStore.getState().windows;

function draft(overrides: Partial<MessageDetail> = {}): MessageDetail {
  return {
    uid: 619,
    subject: "The product, start to finish, in three minutes",
    from_name: "CX Ventures",
    from_email: "chris@cxventures.io",
    to_list: [{ name: null, email: "dana@northwind.example" }],
    cc_list: [],
    bcc_list: [],
    date: "2026-08-25T14:00:00+00:00",
    plain_text: "Hi Dana,",
    sanitized_html: "<p>Hi Dana,</p>",
    attachments: [],
    is_read: true,
    is_flagged: false,
    list_unsubscribe: null,
    list_unsubscribe_post: null,
    message_id: "<x@cxmail.app>",
    references: null,
    in_reply_to: null,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  useWindowStore.setState({ windows: [], nextZIndex: 100 });
  vi.clearAllMocks();
});

describe("openDraftForEdit is idempotent", () => {
  // THE regression (2026-08-25): the open is an IMAP round trip, the user saw
  // nothing happen and clicked again, and every click that resolved opened its
  // own compose window. A double-click alone is three opens (click, click,
  // dblclick), so this is the common path, not an edge.
  it("every click that lands while the draft is loading joins one open — one fetch, one window", async () => {
    const load = deferred<MessageDetail>();
    fetchBody.mockReturnValue(load.promise);

    const clicks = [
      openDraftForEdit(A, "Drafts", 619),
      openDraftForEdit(A, "Drafts", 619),
      openDraftForEdit(A, "Drafts", 619),
    ];
    expect(fetchBody).toHaveBeenCalledTimes(1);
    expect(windows()).toHaveLength(0);

    load.resolve(draft());
    const ids = await Promise.all(clicks);
    expect(ids[0]).not.toBeNull();
    expect(new Set(ids).size).toBe(1);
    expect(windows()).toHaveLength(1);
    expect(windows()[0].key).toBe(draftWindowKey(A, "Drafts", 619));
    expect(windows()[0].type).toBe("compose");
  });

  it("a click on a draft that is already open focuses its window and fetches nothing", async () => {
    fetchBody.mockResolvedValue(draft());
    const first = await openDraftForEdit(A, "Drafts", 619);
    expect(first).not.toBeNull();
    // Bury it: something else on top, then minimized.
    useWindowStore.getState().openWindow({ type: "compose", title: "other", props: {} });
    useWindowStore.getState().minimizeWindow(first!);
    fetchBody.mockClear();

    const again = await openDraftForEdit(A, "Drafts", 619);
    expect(again).toBe(first);
    expect(fetchBody).not.toHaveBeenCalled();
    expect(windows()).toHaveLength(2);
    const w = windows().find((x) => x.id === first)!;
    expect(w.isMinimized).toBe(false);
    expect(w.zIndex).toBe(Math.max(...windows().map((x) => x.zIndex)));
  });

  it("different drafts are different windows", async () => {
    fetchBody.mockResolvedValue(draft());
    const a = await openDraftForEdit(A, "Drafts", 619);
    const b = await openDraftForEdit(A, "Drafts", 620);
    const c = await openDraftForEdit("acct-other", "Drafts", 619);
    expect(new Set([a, b, c]).size).toBe(3);
    expect(windows()).toHaveLength(3);
  });

  it("a failed load opens nothing, reaches the Rust log, and does not poison the next click", async () => {
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    fetchBody.mockRejectedValueOnce(new Error("IMAP error: Connection timed out"));

    expect(await openDraftForEdit(A, "Drafts", 619)).toBeNull();
    expect(windows()).toHaveLength(0);
    expect(logClientError).toHaveBeenCalledWith("draft-open", expect.stringContaining("Connection timed out"));

    // The in-flight slot was released: the retry fetches again and opens.
    fetchBody.mockResolvedValueOnce(draft());
    expect(await openDraftForEdit(A, "Drafts", 619)).not.toBeNull();
    expect(fetchBody).toHaveBeenCalledTimes(2);
    expect(windows()).toHaveLength(1);
    quiet.mockRestore();
  });

  it("real attachments are reloaded exactly once per open; inline-only drafts skip the round trip", async () => {
    const pdf: AttachmentMeta = {
      filename: "deck.pdf",
      content_type: "application/pdf",
      size_bytes: 1024,
      content_id: null,
      is_inline: false,
    };
    const bytes = [{ filename: "deck.pdf", content_type: "application/pdf", data_base64: "AAAA" }];
    fetchBody.mockResolvedValue(draft({ attachments: [pdf] }));
    fetchAttachments.mockResolvedValue(bytes);

    await Promise.all([openDraftForEdit(A, "Drafts", 619), openDraftForEdit(A, "Drafts", 619)]);
    expect(fetchAttachments).toHaveBeenCalledTimes(1);
    expect(fetchAttachments).toHaveBeenCalledWith(A, "Drafts", 619);
    expect(windows()[0].props.defaultAttachments).toEqual(bytes);

    fetchAttachments.mockClear();
    fetchBody.mockResolvedValue(draft({ attachments: [{ ...pdf, is_inline: true, content_id: "img1" }] }));
    await openDraftForEdit(A, "Drafts", 620);
    expect(fetchAttachments).not.toHaveBeenCalled();
    expect(windows()[1].props.defaultAttachments).toEqual([]);
  });
});
