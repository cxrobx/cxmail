import { describe, it, expect, afterEach, vi } from "vitest";
import { render, act } from "@testing-library/react";
import EmailFrame from "@/components/mail/EmailFrame";
import { useUIStore } from "@/stores/uiStore";
import { useResolvedTheme } from "../useResolvedTheme";

vi.mock("@/lib/tauri", () => ({
  api: { imageTrust: { isTrusted: vi.fn().mockResolvedValue(false), trust: vi.fn() } },
}));

// jsdom has no matchMedia; install one whose answer we control and can flip.
function installMatchMedia(initiallyDark: boolean) {
  let dark = initiallyDark;
  const listeners = new Set<() => void>();
  window.matchMedia = ((query: string) => ({
    get matches() {
      return query.includes("dark") ? dark : false;
    },
    media: query,
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
  })) as unknown as typeof window.matchMedia;
  return (next: boolean) => {
    dark = next;
    listeners.forEach((fn) => fn());
  };
}

function Probe() {
  return <span data-testid="t">{useResolvedTheme()}</span>;
}

const DARK_INK = "rgb(232, 220, 200) !important";

afterEach(() => {
  // @ts-expect-error jsdom has none; remove ours
  delete window.matchMedia;
  useUIStore.setState({ theme: "dark" });
});

describe("useResolvedTheme", () => {
  it("resolves `system` to the macOS appearance and follows a live flip", () => {
    const flip = installMatchMedia(true);
    useUIStore.setState({ theme: "system" });
    const { getByTestId } = render(<Probe />);
    expect(getByTestId("t").textContent).toBe("dark");
    act(() => flip(false));
    expect(getByTestId("t").textContent).toBe("light");
  });

  it("an explicit preference wins over the system", () => {
    installMatchMedia(true);
    useUIStore.setState({ theme: "light" });
    const { getByTestId } = render(<Probe />);
    expect(getByTestId("t").textContent).toBe("light");
  });
});

describe("EmailFrame under the `system` preference", () => {
  // The bug: `theme === "dark"` against the PREFERENCE took the light branch
  // under `system`, printing dark ink on dark glass.
  it("paints the dark stylesheet when macOS is dark", () => {
    installMatchMedia(true);
    useUIStore.setState({ theme: "system" });
    const { container } = render(<EmailFrame html="<p>hi</p>" />);
    const srcdoc = container.querySelector("iframe")!.getAttribute("srcdoc")!;
    expect(srcdoc).toContain(DARK_INK);
  });

  it("paints the light stylesheet when macOS is light", () => {
    installMatchMedia(false);
    useUIStore.setState({ theme: "system" });
    const { container } = render(<EmailFrame html="<p>hi</p>" />);
    const srcdoc = container.querySelector("iframe")!.getAttribute("srcdoc")!;
    expect(srcdoc).not.toContain(DARK_INK);
  });
});
