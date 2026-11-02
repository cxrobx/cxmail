import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "@/lib/tauri";
import HerdrHostSection from "../HerdrHostSection";

vi.mock("@/lib/tauri", () => ({
  api: { system: {
    getHerdrHost: vi.fn(),
    setHerdrHost: vi.fn(),
    logClientError: vi.fn(),
  } },
}));

async function field() {
  render(<HerdrHostSection />);
  const input = screen.getByLabelText("Herdr host") as HTMLInputElement;
  await waitFor(() => expect(input).toBeEnabled());
  return input;
}

describe("HerdrHostSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.system.getHerdrHost).mockResolvedValue("");
    vi.mocked(api.system.setHerdrHost).mockImplementation(async (host) => host.trim());
  });

  it("shows blank as this Mac and explains the field", async () => {
    const input = await field();
    expect(input.value).toBe("");
    expect(input.placeholder).toBe("This Mac");
    expect(input).toHaveAccessibleDescription(/SSH target such as user@host/);
  });

  it("shows the saved host", async () => {
    vi.mocked(api.system.getHerdrHost).mockResolvedValue("user@host");
    const input = await field();
    expect(input.value).toBe("user@host");
  });

  it("saves on blur and shows the trimmed value that came back", async () => {
    const input = await field();
    fireEvent.change(input, { target: { value: "  user@host " } });
    fireEvent.blur(input);
    await waitFor(() => expect(input.value).toBe("user@host"));
    expect(api.system.setHerdrHost).toHaveBeenCalledWith("  user@host ");
  });

  it("saves on Enter, and leaving an unchanged field saves nothing", async () => {
    const input = await field();
    fireEvent.change(input, { target: { value: "user@host" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(api.system.setHerdrHost).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(input.value).toBe("user@host"));
    fireEvent.blur(input);
    expect(api.system.setHerdrHost).toHaveBeenCalledTimes(1);
  });

  it("can be cleared back to this Mac", async () => {
    vi.mocked(api.system.getHerdrHost).mockResolvedValue("user@host");
    const input = await field();
    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input);
    await waitFor(() => expect(api.system.setHerdrHost).toHaveBeenCalledWith(""));
  });

  it("keeps a refused value in the field and says why", async () => {
    vi.mocked(api.system.setHerdrHost).mockRejectedValueOnce("That isn't an SSH target.");
    const input = await field();
    fireEvent.change(input, { target: { value: "two words" } });
    fireEvent.blur(input);
    expect(await screen.findByRole("alert")).toHaveTextContent("That isn't an SSH target.");
    expect(input.value).toBe("two words");
  });

  it("never saves over a value it could not load", async () => {
    vi.mocked(api.system.getHerdrHost).mockRejectedValueOnce(new Error("unreadable file"));
    render(<HerdrHostSection />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load");
    const input = screen.getByLabelText("Herdr host");
    expect(input).toBeDisabled();
    fireEvent.blur(input);
    expect(api.system.setHerdrHost).not.toHaveBeenCalled();
  });
});
