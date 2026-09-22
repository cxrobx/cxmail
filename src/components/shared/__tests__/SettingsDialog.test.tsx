import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import SettingsDialog from "@/components/shared/SettingsDialog";
import { useUIStore, TRANSPARENCY_DEFAULT } from "@/stores/uiStore";

/**
 * The Settings dialog is the only surface where the transparency dial exists,
 * so "does it render and are the controls actually wired" is not a formality —
 * a slider bound to nothing looks identical to one bound to something until you
 * drag it.
 */

const reset = () =>
  useUIStore.setState({
    settingsOpen: true,
    theme: "dark",
    density: "comfortable",
    transparency: TRANSPARENCY_DEFAULT,
    emailTransparency: TRANSPARENCY_DEFAULT,
  });

describe("SettingsDialog", () => {
  beforeEach(reset);

  it("renders nothing at all while closed", () => {
    useUIStore.setState({ settingsOpen: false });
    const { container } = render(<SettingsDialog />);
    expect(container).toBeEmptyDOMElement();
  });

  it("drives the store from the transparency slider", () => {
    render(<SettingsDialog />);
    const slider = screen.getByLabelText("Window transparency") as HTMLInputElement;
    // The control is a percentage; the store is a 0–1 fraction. Getting that
    // conversion backwards would clamp every drag to fully-glass.
    expect(slider.value).toBe(String(Math.round(TRANSPARENCY_DEFAULT * 100)));
    fireEvent.change(slider, { target: { value: "70" } });
    expect(useUIStore.getState().transparency).toBeCloseTo(0.7, 5);
    // …and the pane alphas actually moved, which is the thing the user sees.
    expect(document.documentElement.style.getPropertyValue("--alpha-pane")).not.toBe("");
    expect(Number(document.documentElement.style.getPropertyValue("--alpha-pane"))).toBeLessThan(1);
  });

  it("reaches the opaque end, and reports it honestly", () => {
    render(<SettingsDialog />);
    fireEvent.change(screen.getByLabelText("Window transparency"), { target: { value: "0" } });
    expect(useUIStore.getState().transparency).toBe(0);
    // 0 is a real destination, not a disabled state — every pane must be fully
    // opaque there or "drag it back" would not undo the effect.
    for (const a of ["--alpha-pane", "--alpha-sidebar", "--alpha-surface"]) {
      expect(Number(document.documentElement.style.getPropertyValue(a))).toBe(1);
    }
    expect(screen.getByText("0%")).toBeInTheDocument();
  });

  it("holds the message body at its own dial without moving the window", () => {
    render(<SettingsDialog />);
    const email = screen.getByLabelText("Email transparency") as HTMLInputElement;
    expect(email.value).toBe(String(Math.round(TRANSPARENCY_DEFAULT * 100)));
    fireEvent.change(email, { target: { value: "0" } });
    expect(useUIStore.getState().emailTransparency).toBe(0);
    expect(useUIStore.getState().transparency).toBe(TRANSPARENCY_DEFAULT);
    const root = document.documentElement.style;
    // Opaque email: the veil covers everything the pane lets through…
    expect(Number(root.getPropertyValue("--alpha-email-veil"))).toBe(1);
    // …while the window around it stays glass.
    expect(Number(root.getPropertyValue("--alpha-pane"))).toBeLessThan(1);
  });

  it("says so when the window caps the email, instead of looking broken", () => {
    render(<SettingsDialog />);
    fireEvent.change(screen.getByLabelText("Window transparency"), { target: { value: "20" } });
    fireEvent.change(screen.getByLabelText("Email transparency"), { target: { value: "60" } });
    expect(screen.getByText("capped at 20%")).toBeInTheDocument();
    // Capped means no veil: the body is exactly as glassy as the window, never more.
    expect(Number(document.documentElement.style.getPropertyValue("--alpha-email-veil"))).toBe(0);
  });

  it("switches theme and density, and marks the active choice", () => {
    render(<SettingsDialog />);

    const light = screen.getByRole("button", { name: "Light" });
    expect(screen.getByRole("button", { name: "Dark" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(light);
    expect(useUIStore.getState().theme).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");

    fireEvent.click(screen.getByRole("button", { name: "Compact" }));
    expect(useUIStore.getState().density).toBe("compact");
  });

  it("offers System, and stores the preference rather than the resolved theme", () => {
    render(<SettingsDialog />);

    fireEvent.click(screen.getByRole("button", { name: "System" }));
    // The store keeps the DEFERRAL. Collapsing it to the resolved value here
    // would look right for one flip and then stop following macOS entirely,
    // because nothing would be left to say "this one tracks the system".
    expect(useUIStore.getState().theme).toBe("system");
    expect(screen.getByRole("button", { name: "System" })).toHaveAttribute("aria-pressed", "true");
    // …and the DOM still gets a paintable theme, never "system".
    expect(["dark", "light"]).toContain(document.documentElement.getAttribute("data-theme"));
  });

  it("re-derives the pane alphas when the theme changes", () => {
    // The floors are theme-dependent, so a theme flip that left the alphas
    // alone would apply dark's floor to a light window — the unreadable case.
    render(<SettingsDialog />);
    fireEvent.change(screen.getByLabelText("Window transparency"), { target: { value: "100" } });
    const inDark = Number(document.documentElement.style.getPropertyValue("--alpha-pane"));
    fireEvent.click(screen.getByRole("button", { name: "Light" }));
    const inLight = Number(document.documentElement.style.getPropertyValue("--alpha-pane"));
    expect(inLight).toBeGreaterThan(inDark);
  });

  it("closes on Escape and on the close button", () => {
    const { unmount } = render(<SettingsDialog />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(useUIStore.getState().settingsOpen).toBe(false);
    unmount();

    reset();
    render(<SettingsDialog />);
    fireEvent.click(screen.getByLabelText("Close settings"));
    expect(useUIStore.getState().settingsOpen).toBe(false);
  });

  it("keeps the panel opaque — a glass panel over a glass window is unreadable", () => {
    const { container } = render(<SettingsDialog />);
    const panel = container.querySelector('[role="dialog"]')!;
    expect(panel.className).toContain("bg-base-solid");
    // `\b` is NOT a usable boundary here — a hyphen is a word boundary, so
    // /\bbg-base\b/ matches inside `bg-base-solid` and the assertion can never
    // pass. Reject only a `bg-base` that ends there.
    expect(panel.className).not.toMatch(/bg-base(?![-\w])/);
  });

  it("moves the DIALOG when its header is dragged, not the OS window", () => {
    // The trap this guards is one attribute wide: `data-tauri-drag-region` on
    // this header would slide CXMail across the desktop instead of moving the
    // dialog, and it looks like a reasonable thing to reach for (gotcha #51).
    render(<SettingsDialog />);
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    const header = screen.getByText("Settings").parentElement as HTMLElement;

    expect(header.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(dialog.style.transform).toBe("translate(0px, 0px)");

    fireEvent.pointerDown(header, { clientX: 100, clientY: 100 });
    fireEvent.pointerMove(window, { clientX: 160, clientY: 140 });
    expect(dialog.style.transform).toBe("translate(60px, 40px)");

    // Released: further movement must not keep dragging it.
    fireEvent.pointerUp(window);
    fireEvent.pointerMove(window, { clientX: 400, clientY: 400 });
    expect(dialog.style.transform).toBe("translate(60px, 40px)");
  });

  it("does not start a drag from the close button", () => {
    render(<SettingsDialog />);
    const close = screen.getByLabelText("Close settings");
    fireEvent.pointerDown(close, { clientX: 10, clientY: 10 });
    fireEvent.pointerMove(window, { clientX: 200, clientY: 200 });
    // Still open and unmoved — the button is a button first.
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(dialog.style.transform).toBe("translate(0px, 0px)");
  });

  it("caps its height and scrolls, so a new section cannot run off the screen", () => {
    // Adding the triage section pushed the dialog past the bottom of a 1440px
    // display and cut off its own button. Growth is now the body's problem.
    render(<SettingsDialog />);
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(dialog.className).toContain("max-h-[80vh]");
    const body = dialog.querySelector(".overflow-y-auto");
    expect(body).not.toBeNull();
  });
});
