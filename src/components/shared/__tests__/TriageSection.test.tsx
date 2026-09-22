import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import SettingsDialog from "@/components/shared/SettingsDialog";
import TriageSection from "@/components/shared/TriageSection";
import { api } from "@/lib/tauri";
import { useUIStore, TRANSPARENCY_DEFAULT } from "@/stores/uiStore";

/**
 * Triage is the first section in this dialog that talks to the backend, which
 * makes it the first that can take the dialog down with it.
 */

const status = (mode: string) => ({
  mode,
  model: "gpt-5.6-luna",
  effort: "max",
  input_tokens: 0,
  output_tokens: 0,
  withheld: 0,
  verdicts: 12,
  today: 3,
  daily_cap: 400,
  pending: 7,
  provider_ready: true,
});

const account = (email: string, triage_enabled: boolean) => ({
  id: email,
  email,
  display_name: null,
  provider: "gmail",
  imap_host: "i",
  imap_port: 993,
  smtp_host: "s",
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
  triage_enabled,
});

beforeEach(() => {
  vi.restoreAllMocks();
  useUIStore.setState({
    settingsOpen: true,
    theme: "dark",
    density: "comfortable",
    transparency: TRANSPARENCY_DEFAULT,
    emailTransparency: TRANSPARENCY_DEFAULT,
  });
});

describe("TriageSection", () => {
  it("does not take the Settings dialog down when the backend has no triage command", async () => {
    // The real failure this guards: an older binary, or a partial upgrade,
    // returns undefined from invoke. Before the fix a bare `.then` threw during
    // render and Theme, Density and both transparency dials went with it —
    // none of which have anything to do with triage.
    vi.spyOn(api.ai, "getTriageStatus").mockReturnValue(
      undefined as unknown as ReturnType<typeof api.ai.getTriageStatus>,
    );
    render(<SettingsDialog />);
    expect(screen.getByLabelText("Window transparency")).toBeTruthy();
    expect(screen.getByText("Theme")).toBeTruthy();
    // The section itself simply is not there, which is the honest fallback.
    await waitFor(() => expect(screen.queryByText("Inbox triage")).toBeNull());
  });

  it("marks the active mode and switches on click", async () => {
    vi.spyOn(api.ai, "getTriageStatus").mockResolvedValue(status("shadow"));
    vi.spyOn(api.accounts, "list").mockResolvedValue([]);
    const setMode = vi.spyOn(api.ai, "setTriageMode").mockResolvedValue("on");

    render(<TriageSection />);
    const shadow = await screen.findByRole("button", { name: "Shadow" });
    expect(shadow.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "On" }).getAttribute("aria-pressed")).toBe("false");

    fireEvent.click(screen.getByRole("button", { name: "On" }));
    await waitFor(() => expect(setMode).toHaveBeenCalledWith("on"));
  });

  it("shows one checkbox per account, ticked from the account's own flag", async () => {
    vi.spyOn(api.ai, "getTriageStatus").mockResolvedValue(status("on"));
    vi.spyOn(api.accounts, "list").mockResolvedValue([
      account("chris@cxventures.io", true),
      account("cxrobx@gmail.com", false),
    ]);
    const setEnabled = vi.spyOn(api.ai, "setAccountTriageEnabled").mockResolvedValue(undefined);

    render(<TriageSection />);
    const business = (await screen.findByLabelText(
      "Triage chris@cxventures.io",
    )) as HTMLInputElement;
    const personal = screen.getByLabelText("Triage cxrobx@gmail.com") as HTMLInputElement;
    expect(business.checked).toBe(true);
    expect(personal.checked).toBe(false);

    fireEvent.click(personal);
    await waitFor(() =>
      expect(setEnabled).toHaveBeenCalledWith("cxrobx@gmail.com", true),
    );
  });

  it("keeps the triage model separate from the one Reply with AI uses", async () => {
    // The whole reason this control exists: `ai:model` drives draft writing,
    // and a model picked to classify cheaply must not also write the email.
    vi.spyOn(api.ai, "getTriageStatus").mockResolvedValue(status("on"));
    vi.spyOn(api.accounts, "list").mockResolvedValue([]);
    const setModel = vi.spyOn(api.ai, "setTriageModel").mockResolvedValue("gpt-5.4-mini");

    render(<TriageSection />);
    const field = (await screen.findByLabelText("Triage model")) as HTMLInputElement;
    expect(field.value).toBe("gpt-5.6-luna");
    fireEvent.blur(field, { target: { value: "gpt-5.4-mini" } });
    await waitFor(() => expect(setModel).toHaveBeenCalledWith("gpt-5.4-mini"));
  });

  it("does not put a reasoning-effort control in front of the user", async () => {
    // Removed deliberately. Nobody can choose between "high" and "medium"
    // without running an experiment and reading token counts, so a dropdown on
    // it is a decision the user cannot make, shown every time they open
    // Settings. It is a constant in the backend with an override key.
    vi.spyOn(api.ai, "getTriageStatus").mockResolvedValue(status("on"));
    vi.spyOn(api.accounts, "list").mockResolvedValue([]);
    render(<TriageSection />);
    await screen.findByLabelText("Triage model");
    expect(screen.queryByLabelText("Reasoning effort")).toBeNull();
  });

  it("cannot run a pass while the mode is off", async () => {
    vi.spyOn(api.ai, "getTriageStatus").mockResolvedValue(status("off"));
    vi.spyOn(api.accounts, "list").mockResolvedValue([account("a@b.io", true)]);
    render(<TriageSection />);
    const run = (await screen.findByRole("button", { name: /Run a pass now/ })) as HTMLButtonElement;
    expect(run.disabled).toBe(true);
  });
});
