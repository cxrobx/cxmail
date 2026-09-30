import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "@/lib/tauri";
import MenuBarSection from "../MenuBarSection";

vi.mock("@/lib/tauri", () => ({
  api: { system: {
    getShowInMenuBar: vi.fn(),
    setShowInMenuBar: vi.fn(),
    logClientError: vi.fn(),
  } },
}));

describe("MenuBarSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.system.getShowInMenuBar).mockResolvedValue(true);
    vi.mocked(api.system.setShowInMenuBar).mockResolvedValue(undefined);
  });

  it("hides and shows the icon through the backend", async () => {
    render(<MenuBarSection />);
    const toggle = screen.getByRole("switch", { name: "Show in menu bar" });
    expect(toggle).toBeDisabled();
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute("aria-checked", "true");
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
    expect(api.system.setShowInMenuBar).toHaveBeenLastCalledWith(false);
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "true"));
    expect(api.system.setShowInMenuBar).toHaveBeenLastCalledWith(true);
  });

  it("restores an off preference when Settings is reopened", async () => {
    vi.mocked(api.system.getShowInMenuBar).mockResolvedValue(false);
    const { unmount } = render(<MenuBarSection />);
    await waitFor(() => expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "false"));
    unmount();
    render(<MenuBarSection />);
    await waitFor(() => expect(screen.getByRole("switch")).toHaveAttribute("aria-checked", "false"));
  });

  it("keeps the previous value and allows retry after a failed save", async () => {
    vi.mocked(api.system.setShowInMenuBar).mockRejectedValueOnce(new Error("disk full"));
    render(<MenuBarSection />);
    const toggle = screen.getByRole("switch");
    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(toggle).toBeEnabled();
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("disables changes if the saved preference could not be loaded", async () => {
    vi.mocked(api.system.getShowInMenuBar).mockRejectedValueOnce(new Error("unreadable file"));
    render(<MenuBarSection />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load");
    expect(screen.getByRole("switch")).toBeDisabled();
    expect(api.system.setShowInMenuBar).not.toHaveBeenCalled();
  });
});
