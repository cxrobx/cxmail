import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import SendAsSection from "@/components/shared/SendAsSection";
import { api } from "@/lib/tauri";
import type { Account, SendAsAddress } from "@/types/email";

const PRIMARY = "sidalias@icloud.com";
const ALIAS = "rileyprime@icloud.com";

const account = { id: "acct", email: PRIMARY, display_name: null } as unknown as Account;

const addr = (email: string, extra: Partial<SendAsAddress> = {}): SendAsAddress => ({
  account_id: "acct",
  email,
  display_name: null,
  signature_html: null,
  is_primary: false,
  identity_id: null,
  ...extra,
});

beforeEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(api.accounts, "list").mockResolvedValue([account]);
  vi.spyOn(api.identities, "suggestSendAs").mockResolvedValue([]);
});

describe("SendAsSection", () => {
  it("labels an alias found on Sent mail, and removes it through the tombstoning call", async () => {
    vi.spyOn(api.identities, "listSendAs").mockResolvedValue([
      addr(PRIMARY, { is_primary: true }),
      addr(ALIAS, { from_sent: true }),
    ]);
    // An alias found in Sent has no identity row: the old `identities.delete`
    // path returned early on `identity_id == null`, so its bin icon did nothing.
    const remove = vi.spyOn(api.identities, "removeSendAs").mockResolvedValue(undefined);
    const del = vi.spyOn(api.identities, "delete");

    render(<SendAsSection />);
    await screen.findByText(ALIAS);
    expect(screen.getByText("found in Sent")).toBeTruthy();

    fireEvent.click(screen.getByLabelText(`Remove ${ALIAS}`));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("acct", ALIAS));
    expect(del).not.toHaveBeenCalled();
  });

  it("never offers the primary for removal", async () => {
    vi.spyOn(api.identities, "listSendAs").mockResolvedValue([addr(PRIMARY, { is_primary: true })]);
    render(<SendAsSection />);
    await screen.findByText(PRIMARY);
    expect(screen.queryByLabelText(`Remove ${PRIMARY}`)).toBeNull();
  });
});
