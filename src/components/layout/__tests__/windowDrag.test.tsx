/**
 * Window dragging is invisible when it is broken — nothing in the DOM is wrong,
 * no error is logged, and the window simply refuses to move. It broke in two
 * independent layers at once (gotcha #51), so this file guards both; each check
 * catches only its own layer and neither implies the other.
 *
 * Layer 1 — the ACL. `drag.js` ends at `invoke('plugin:window|start_dragging')`,
 * an ordinary ACL-gated command that `core:window:default` does NOT grant.
 * Layer 2 — the markup. Asserting the attribute is merely *present* proves
 * nothing: a bare attribute and `deep` look identical in the DOM and behave
 * completely differently, so this ports the real `isDragRegion` from
 * `tauri-<ver>/src/window/scripts/drag.js` and probes actual rendered nodes.
 */
import { render } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AppLayout from "@/components/layout/AppLayout";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";
import { useWindowStore } from "@/stores/windowStore";

vi.mock("@/components/layout/Sidebar", () => ({ default: () => <div>Sidebar</div> }));
vi.mock("@/components/layout/StatusBar", () => ({ default: () => <div>StatusBar</div> }));
vi.mock("@/components/mail/MessageList", () => ({ default: () => <div>MessageList</div> }));
vi.mock("@/components/mail/ReadingPane", () => ({ default: () => <div>ReadingPane</div> }));
vi.mock("@/components/mail/ComposeModal", () => ({ default: () => <div>ComposeModal</div> }));
vi.mock("@/components/mail/SearchBar", () => ({ default: () => <div>SearchBar</div> }));
vi.mock("@/components/shared/CommandPalette", () => ({ default: () => <div>CommandPalette</div> }));
vi.mock("@/components/shared/KeyboardShortcutSheet", () => ({ default: () => <div>KeyboardShortcutSheet</div> }));
vi.mock("@/components/shared/McpApprovalModal", () => ({ default: () => <div>McpApprovalModal</div> }));
vi.mock("@/components/shared/UndoSendToast", () => ({ default: () => <div>UndoSendToast</div> }));
vi.mock("@/components/shared/Toast", () => ({ default: () => <div>Toast</div> }));
vi.mock("@/components/shared/FloatingWindowManager", () => ({ default: () => <div>FloatingWindowManager</div> }));
vi.mock("@/components/accounts/AccountSetup", () => ({ default: () => <div>AccountSetup</div> }));
vi.mock("@/hooks/useKeyboardShortcuts", () => ({ useKeyboardShortcuts: () => {} }));
vi.mock("@/lib/tauri", () => ({
  api: {
    messages: {
      sync: vi.fn(),
      syncAllInboxes: vi.fn(),
      listUnsubscribedSenders: vi.fn(() => Promise.resolve([])),
    },
  },
}));

/* ------------------------------------------------------------------ *
 * Ported verbatim from tauri-2.11.5/src/window/scripts/drag.js.
 * Do not "simplify" it — the `el === composedPath[0]` line IS the
 * bare-vs-deep distinction this file exists to pin.
 * ------------------------------------------------------------------ */
const CLICKABLE_TAGS = new Set(["A", "BUTTON", "INPUT", "SELECT", "TEXTAREA", "LABEL", "SUMMARY"]);
const INTERACTIVE_ROLES = new Set([
  "button", "link", "menuitem", "tab", "checkbox", "radio", "switch", "option",
]);

function isClickableElement(el: HTMLElement): boolean {
  return (
    CLICKABLE_TAGS.has(el.tagName) ||
    (el.hasAttribute("contenteditable") && el.getAttribute("contenteditable") !== "false") ||
    (el.hasAttribute("tabindex") && el.getAttribute("tabindex") !== "-1") ||
    INTERACTIVE_ROLES.has(el.getAttribute("role") ?? "")
  );
}

function isDragRegion(composedPath: HTMLElement[]): boolean {
  for (const el of composedPath) {
    const attr = el.getAttribute("data-tauri-drag-region");
    if (isClickableElement(el) && attr === null) return false;
    if (attr === null) continue;
    if (attr === "false") return false;
    if (attr === "deep") return true;
    if (attr === "" || attr === "true") return el === composedPath[0];
  }
  return false;
}

/** jsdom has no shadow DOM here, so the ancestor chain IS the composed path. */
function pathFrom(el: HTMLElement): HTMLElement[] {
  const path: HTMLElement[] = [];
  let node: HTMLElement | null = el;
  while (node) {
    path.push(node);
    node = node.parentElement;
  }
  return path;
}

/** Would a mousedown landing on this element move the window? */
function dragsWindow(el: HTMLElement): boolean {
  return isDragRegion(pathFrom(el));
}

const sampleAccount = {
  id: "acc-1",
  email: "person@example.com",
  display_name: "Personal",
  provider: "gmail",
  imap_host: "imap.gmail.com",
  imap_port: 993,
  smtp_host: "smtp.gmail.com",
  smtp_port: 587,
  imap_security: "implicit",
  smtp_security: "starttls",
  imap_username: null,
  smtp_username: null,
  color: null,
  is_active: true,
  sort_order: 0,
  group_name: null,
  notify_enabled: true,
  track_opens_enabled: false,
  hidden_from_aggregates: false,
  triage_enabled: false,
};

describe("titlebar drag region — markup layer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.messages.listUnsubscribedSenders).mockResolvedValue([]);
    useAccountStore.setState({ accounts: [sampleAccount], isSetupComplete: true, showingAddAccount: false });
    useMailStore.setState({
      isComposing: false, isSyncing: false, syncError: null, lastSyncedAt: null,
      selectedAccountId: null, selectedFolder: null, messages: [],
    });
    useWindowStore.setState({ windows: [], nextZIndex: 100 });
  });

  function titlebar(container: HTMLElement): HTMLElement {
    const strip = container.querySelector<HTMLElement>("[data-tauri-drag-region]");
    if (!strip) throw new Error("no drag region in the layout at all");
    return strip;
  }

  it("drags from the strip itself", () => {
    const { container } = render(<AppLayout />);
    expect(dragsWindow(titlebar(container))).toBe(true);
  });

  it("drags from the app title, which is a DESCENDANT of the strip", () => {
    // The mutation guard: revert `deep` to a bare attribute and only this fails.
    // A bare attribute leaves every child of the strip a dead spot, which reads
    // as "dragging works in some places" rather than as a bug.
    const { container, getByText } = render(<AppLayout />);
    const title = getByText("CXMail");
    expect(titlebar(container).contains(title)).toBe(true);
    expect(dragsWindow(title as HTMLElement)).toBe(true);
  });

  it("does NOT drag from the Compose button inside the strip", () => {
    // No opt-out markup is needed for this — isClickableElement excludes BUTTON.
    // If this ever fails, someone added an opt-in attribute to a control.
    const { getByTitle } = render(<AppLayout />);
    expect(dragsWindow(getByTitle("Compose (Cmd+N)") as HTMLElement)).toBe(false);
  });

  it("does NOT drag from content below the strip", () => {
    const { getByText } = render(<AppLayout />);
    expect(dragsWindow(getByText("Sidebar") as HTMLElement)).toBe(false);
  });
});

describe("titlebar drag region — ACL layer", () => {
  it("grants core:window:allow-start-dragging", () => {
    // Correct markup with this grant missing produces a drag that is authorized
    // away: the invoke is rejected, nothing is logged, and only the sliver of
    // native titlebar the webview does not cover still moves the window.
    // `core:window:default` grants read-only getters plus
    // allow-internal-toggle-maximize — NOT this. Double-click-to-maximize
    // working while dragging does not is the tell that you are on this layer.
    const path = resolve(__dirname, "../../../../src-tauri/capabilities/default.json");
    const capability = JSON.parse(readFileSync(path, "utf8")) as { permissions: unknown[] };
    expect(capability.permissions).toContain("core:window:allow-start-dragging");
  });
});
