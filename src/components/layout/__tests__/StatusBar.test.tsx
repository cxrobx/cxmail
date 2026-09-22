import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import StatusBar from "@/components/layout/StatusBar";
import { useAccountStore } from "@/stores/accountStore";
import { useMailStore } from "@/stores/mailStore";

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

describe("StatusBar", () => {
  beforeEach(() => {
    useAccountStore.setState({
      accounts: [sampleAccount],
      isSetupComplete: true,
      showingAddAccount: false,
    });
    useMailStore.setState({
      isSyncing: false,
      syncError: "Mailbox timeout while checking INBOX",
      selectedAccountId: "acc-1",
      selectedFolder: "INBOX",
      messages: [],
      lastSyncedAt: null,
    });
  });

  it("renders the actual sync error text", () => {
    render(<StatusBar />);

    expect(screen.getByText("Mailbox timeout while checking INBOX")).toBeInTheDocument();
  });
});
