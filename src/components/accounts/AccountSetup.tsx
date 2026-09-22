import { useState, useEffect, useRef } from "react";
import { api } from "@/lib/tauri";
import { useAccountStore } from "@/stores/accountStore";
import { listen } from "@tauri-apps/api/event";
import { Mail, Loader2, ArrowLeft, ExternalLink, ChevronDown, ChevronRight, Server } from "lucide-react";
import type { Account, ImapAccountSettings, DiscoveredConfig } from "@/types/email";
import { presetForEmail, type MailPreset } from "@/lib/mailPresets";

type SetupView = "providers" | "icloud-form" | "imap-form";

const inputClass =
  "w-full rounded-lg border border-border bg-surface px-3 py-2.5 text-sm text-content placeholder-content-muted outline-none transition-colors focus:border-accent disabled:opacity-50";

export default function AccountSetup() {
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<SetupView>("providers");
  const [icloudEmail, setIcloudEmail] = useState("");
  const [icloudPassword, setIcloudPassword] = useState("");
  const [imapEmail, setImapEmail] = useState("");
  const [imapPassword, setImapPassword] = useState("");
  // Server settings stay collapsed on the preset path — the whole point is
  // that a known domain needs two fields. It opens itself when the preset
  // lookup misses, or when verification failed and the settings are suspect.
  const [showServerSettings, setShowServerSettings] = useState(false);
  const [server, setServer] = useState({
    imap_host: "",
    imap_port: "",
    smtp_host: "",
    smtp_port: "",
    imap_username: "",
    smtp_username: "",
  });
  const { addAccount } = useAccountStore();

  const preset: MailPreset | null = presetForEmail(imapEmail);
  const [discovery, setDiscovery] = useState<DiscoveredConfig | null>(null);
  const [discoveryState, setDiscoveryState] =
    useState<"idle" | "looking" | "found" | "none">("idle");
  // Monotonic sequence guards against a slow lookup for an older address
  // landing after a newer one and silently prefilling the wrong server.
  const discoverySeq = useRef(0);

  useEffect(() => {
    const email = imapEmail.trim();
    if (view !== "imap-form" || !email.includes("@") || email.endsWith("@")) {
      setDiscovery(null);
      setDiscoveryState("idle");
      return;
    }
    const seq = ++discoverySeq.current;
    setDiscoveryState("looking");
    const timer = setTimeout(async () => {
      try {
        const found = await api.auth.discoverMailConfig(email);
        if (seq !== discoverySeq.current) return; // a newer address superseded this
        if (!found) {
          setDiscovery(null);
          setDiscoveryState("none");
          return;
        }
        setDiscovery(found);
        setDiscoveryState("found");
        // A curated preset keeps the two-field flow. Anything discovered over
        // the network gets shown: the user must be able to see the hostname
        // before a password is sent to it (DNS is spoofable without DNSSEC).
        if (found.source !== "preset") {
          setServer((prev) => ({
            ...prev,
            imap_host: found.imap_host,
            imap_port: String(found.imap_port),
            smtp_host: found.smtp_host,
            smtp_port: String(found.smtp_port),
            imap_username: found.imap_username ?? "",
            smtp_username: found.smtp_username ?? "",
          }));
          setShowServerSettings(true);
        }
      } catch {
        if (seq === discoverySeq.current) setDiscoveryState("none");
      }
    }, 500);
    return () => clearTimeout(timer);
  }, [imapEmail, view]);

  useEffect(() => {
    const unlisten = listen<Account>("account-added", (event) => {
      addAccount(event.payload);
      setIsLoading(false);
      setView("providers");
      setIcloudEmail("");
      setIcloudPassword("");
      setImapEmail("");
      setImapPassword("");
      setShowServerSettings(false);
      setDiscovery(null);
      setDiscoveryState("idle");
      setServer({ imap_host: "", imap_port: "", smtp_host: "", smtp_port: "", imap_username: "", smtp_username: "" });
    });

    const unlistenError = listen<string>("oauth-error", (event) => {
      setError(event.payload);
      setIsLoading(false);
    });

    return () => {
      unlisten.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, [addAccount]);

  const handleConnectGmail = async () => {
    setIsLoading(true);
    setError(null);
    try {
      await api.auth.startOAuth2("gmail");
    } catch (e) {
      setError(String(e));
      setIsLoading(false);
    }
  };

  const handleConnectOutlook = async () => {
    setIsLoading(true);
    setError(null);
    try {
      await api.auth.startOAuth2("outlook");
    } catch (e) {
      setError(String(e));
      setIsLoading(false);
    }
  };

  const handleConnectICloud = async () => {
    if (!icloudEmail.trim() || !icloudPassword.trim()) {
      setError("Email and app-specific password are required.");
      return;
    }
    setIsLoading(true);
    setError(null);
    try {
      await api.auth.addICloudAccount(icloudEmail.trim(), icloudPassword.trim());
    } catch (e) {
      setError(String(e));
      setIsLoading(false);
    }
  };


  const handleConnectImap = async () => {
    const email = imapEmail.trim();
    const password = imapPassword.trim();
    if (!email || !password) {
      setError("Email and password are required.");
      return;
    }
    setIsLoading(true);
    setError(null);
    // Send only what the user actually typed. Anything omitted is resolved by
    // the backend from the preset (or refused) — the UI never invents a host.
    const settings: ImapAccountSettings = {
      preset_id: preset?.id ?? null,
      imap_host: server.imap_host.trim() || null,
      imap_port: server.imap_port.trim() ? Number(server.imap_port) : null,
      smtp_host: server.smtp_host.trim() || null,
      smtp_port: server.smtp_port.trim() ? Number(server.smtp_port) : null,
      imap_username: server.imap_username.trim() || null,
      smtp_username: server.smtp_username.trim() || null,
      // Only pass security when it still describes the port on screen. If the
      // user retyped the port, the backend derives it instead of trusting a
      // stale value from discovery.
      imap_security:
        discovery && server.imap_port.trim() === String(discovery.imap_port)
          ? discovery.imap_security
          : null,
      smtp_security:
        discovery && server.smtp_port.trim() === String(discovery.smtp_port)
          ? discovery.smtp_security
          : null,
    };
    try {
      await api.auth.addImapAccount(email, password, settings);
      // add_imap_account emits "account-added" on success; the listener above
      // clears loading and returns to the picker.
    } catch (e) {
      setError(String(e));
      setIsLoading(false);
      // The failure is almost always a wrong host/port, so surface the fields
      // the user needs rather than making them find the disclosure.
      setShowServerSettings(true);
    }
  };

  if (view === "imap-form") {
    const serverField = (
      key: keyof typeof server,
      label: string,
      placeholder: string,
      type = "text",
    ) => (
      <div>
        <label htmlFor={`imap-${key}`} className="mb-1 block text-xs font-medium text-content-secondary">
          {label}
        </label>
        <input
          id={`imap-${key}`}
          type={type}
          value={server[key]}
          onChange={(e) => setServer({ ...server, [key]: e.target.value })}
          placeholder={placeholder}
          disabled={isLoading}
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          className={inputClass}
        />
      </div>
    );

    return (
      <div className="flex w-full items-center justify-center bg-base py-8">
        <div className="w-[400px]">
          <button
            onClick={() => { setView("providers"); setError(null); }}
            className="mb-6 flex items-center gap-1.5 text-sm text-content-secondary transition-colors hover:text-content"
          >
            <ArrowLeft className="h-4 w-4" />
            Back
          </button>

          <div className="text-center">
            <div className="mx-auto mb-6 flex h-16 w-16 items-center justify-center rounded-2xl bg-content-muted/10">
              <Server className="h-8 w-8 text-content-secondary" />
            </div>
            <h1 className="text-2xl font-semibold text-content">Other mail account</h1>
            <p className="mt-2 text-sm text-content-secondary">
              Connect any mailbox that speaks IMAP and SMTP.
            </p>
          </div>

          <div className="mt-8 space-y-4">
            <div>
              <label htmlFor="imap-email" className="mb-1.5 block text-sm font-medium text-content-secondary">
                Email Address
              </label>
              <input
                id="imap-email"
                type="email"
                value={imapEmail}
                onChange={(e) => setImapEmail(e.target.value)}
                placeholder="you@example.com"
                disabled={isLoading}
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                className={inputClass}
                onKeyDown={(e) => e.key === "Enter" && document.getElementById("imap-password")?.focus()}
              />
              {discoveryState === "looking" ? (
                <p className="mt-1.5 flex items-center gap-1.5 text-xs text-content-muted">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  Looking up server settings…
                </p>
              ) : discoveryState === "found" && discovery ? (
                <p className="mt-1.5 text-xs text-content-secondary">
                  Recognized as{" "}
                  <span className="text-content">
                    {discovery.display_name ?? discovery.imap_host}
                  </span>
                  {discovery.source === "preset" || discovery.source === "ispdb"
                    ? " — server settings filled in automatically."
                    : discovery.source === "autoconfig"
                      ? " — settings published by your own mail domain."
                      : " — settings found in your domain's DNS. Check them below before connecting."}
                </p>
              ) : discoveryState === "none" ? (
                <p className="mt-1.5 text-xs text-content-secondary">
                  Couldn't find settings for this domain — enter them below.
                </p>
              ) : null}
            </div>

            <div>
              <label htmlFor="imap-password" className="mb-1.5 block text-sm font-medium text-content-secondary">
                Password
              </label>
              <input
                id="imap-password"
                type="password"
                value={imapPassword}
                onChange={(e) => setImapPassword(e.target.value)}
                placeholder="••••••••••••"
                disabled={isLoading}
                className={inputClass}
                onKeyDown={(e) => e.key === "Enter" && handleConnectImap()}
              />
              <p className="mt-1.5 text-xs text-content-muted">
                {discovery?.hint ?? preset?.hint ?? "Most providers require an app-specific password rather than your account password."}
              </p>
            </div>

            <div>
              <button
                type="button"
                onClick={() => setShowServerSettings((v) => !v)}
                className="flex items-center gap-1 text-xs font-medium text-content-secondary transition-colors hover:text-content"
              >
                {showServerSettings || discoveryState === "none" ? (
                  <ChevronDown className="h-3.5 w-3.5" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5" />
                )}
                Server settings
                {discovery && !showServerSettings ? " (optional)" : ""}
              </button>

              {(showServerSettings || discoveryState === "none") && (
                <div className="mt-3 space-y-3 rounded-lg border border-border p-3">
                  <div className="grid grid-cols-[1fr_84px] gap-2">
                    {serverField("imap_host", "Incoming (IMAP) server", discovery?.imap_host ?? preset?.imap_host ?? "imap.example.com")}
                    {serverField("imap_port", "Port", String(discovery?.imap_port ?? preset?.imap_port ?? 993))}
                  </div>
                  <div className="grid grid-cols-[1fr_84px] gap-2">
                    {serverField("smtp_host", "Outgoing (SMTP) server", discovery?.smtp_host ?? preset?.smtp_host ?? "smtp.example.com")}
                    {serverField("smtp_port", "Port", String(discovery?.smtp_port ?? preset?.smtp_port ?? 587))}
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    {serverField("imap_username", "IMAP username", "same as email")}
                    {serverField("smtp_username", "SMTP username", "same as IMAP")}
                  </div>
                  <p className="text-xs text-content-muted">
                    Encryption is chosen from the port: 993 / 465 use TLS, 587 uses STARTTLS.
                    Connections are always encrypted.
                  </p>
                </div>
              )}
            </div>

            <button
              onClick={handleConnectImap}
              disabled={isLoading || !imapEmail.trim() || !imapPassword.trim()}
              className="flex w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 py-3 text-sm font-medium text-content transition-colors hover:bg-accent-hover disabled:opacity-50"
            >
              {isLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {isLoading ? "Verifying incoming and outgoing…" : "Connect account"}
            </button>
          </div>

          {error && (
            <div className="mt-4 rounded-lg bg-red-500/10 p-3 text-sm text-red-400">
              {error}
              <button onClick={() => setError(null)} className="ml-2 underline">
                Dismiss
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  if (view === "icloud-form") {
    return (
      <div className="flex w-full items-center justify-center bg-base py-8">
        <div className="w-[400px]">
          <button
            onClick={() => { setView("providers"); setError(null); }}
            className="mb-6 flex items-center gap-1 text-sm text-content-secondary transition-colors hover:text-content"
          >
            <ArrowLeft className="h-4 w-4" />
            Back
          </button>

          <div className="text-center">
            <div className="mx-auto mb-6 flex h-16 w-16 items-center justify-center rounded-2xl bg-content-muted/10">
              <Mail className="h-8 w-8 text-content-secondary" />
            </div>
            <h1 className="text-2xl font-semibold text-content">Connect iCloud</h1>
            <p className="mt-2 text-sm text-content-secondary">
              iCloud requires an{" "}
              <a
                href="https://support.apple.com/en-us/102654"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-0.5 text-accent hover:underline"
              >
                app-specific password
                <ExternalLink className="h-3 w-3" />
              </a>
              {" "}from your Apple ID settings.
            </p>
          </div>

          <div className="mt-8 space-y-4">
            <div>
              <label htmlFor="icloud-email" className="mb-1.5 block text-sm font-medium text-content-secondary">
                Apple ID / iCloud Email
              </label>
              <input
                id="icloud-email"
                type="email"
                value={icloudEmail}
                onChange={(e) => setIcloudEmail(e.target.value)}
                placeholder="you@icloud.com"
                disabled={isLoading}
                className="w-full rounded-lg border border-border bg-surface px-3 py-2.5 text-sm text-content placeholder-content-muted outline-none transition-colors focus:border-accent disabled:opacity-50"
                onKeyDown={(e) => e.key === "Enter" && document.getElementById("icloud-password")?.focus()}
              />
            </div>
            <div>
              <label htmlFor="icloud-password" className="mb-1.5 block text-sm font-medium text-content-secondary">
                App-Specific Password
              </label>
              <input
                id="icloud-password"
                type="password"
                value={icloudPassword}
                onChange={(e) => setIcloudPassword(e.target.value)}
                placeholder="xxxx-xxxx-xxxx-xxxx"
                disabled={isLoading}
                className="w-full rounded-lg border border-border bg-surface px-3 py-2.5 text-sm text-content placeholder-content-muted outline-none transition-colors focus:border-accent disabled:opacity-50"
                onKeyDown={(e) => e.key === "Enter" && handleConnectICloud()}
              />
            </div>

            <button
              onClick={handleConnectICloud}
              disabled={isLoading || !icloudEmail.trim() || !icloudPassword.trim()}
              className="flex w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 py-3 text-sm font-medium text-content transition-colors hover:bg-accent-hover disabled:opacity-50"
            >
              {isLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {isLoading ? "Connecting..." : "Connect iCloud"}
            </button>
          </div>

          {error && (
            <div className="mt-4 rounded-lg bg-red-500/10 p-3 text-sm text-red-400">
              {error}
              <button
                onClick={() => setError(null)}
                className="ml-2 underline"
              >
                Dismiss
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="flex w-full items-center justify-center bg-base py-8">
      <div className="w-[400px] text-center">
        <div className="mx-auto mb-6 flex h-16 w-16 items-center justify-center rounded-2xl bg-accent/10">
          <Mail className="h-8 w-8 text-accent" />
        </div>
        <h1 className="text-2xl font-semibold text-content">Welcome to CXMail</h1>
        <p className="mt-2 text-sm text-content-secondary">
          Connect your email account to get started.
        </p>

        <div className="mt-8 space-y-3">
          <button
            onClick={handleConnectGmail}
            disabled={isLoading}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 py-3 text-sm font-medium text-white transition-colors hover:bg-accent-hover disabled:opacity-50"
          >
            {isLoading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : null}
            {isLoading ? "Waiting for authentication..." : "Connect Gmail"}
          </button>

          <button
            onClick={() => { setView("icloud-form"); setError(null); }}
            disabled={isLoading}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-surface px-4 py-3 text-sm font-medium text-content-secondary transition-colors hover:bg-elevated disabled:opacity-50"
          >
            Connect iCloud
          </button>

          <button
            onClick={handleConnectOutlook}
            disabled={isLoading}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-surface px-4 py-3 text-sm font-medium text-content-secondary transition-colors hover:bg-elevated disabled:opacity-50"
          >
            {isLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
            Connect Outlook
          </button>

          <button
            onClick={() => { setView("imap-form"); setError(null); }}
            disabled={isLoading}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-surface px-4 py-3 text-sm font-medium text-content-secondary transition-colors hover:bg-elevated disabled:opacity-50"
          >
            Other mail account
          </button>
        </div>

        {error && (
          <div className="mt-4 rounded-lg bg-red-500/10 p-3 text-sm text-red-400">
            {error}
            <button
              onClick={() => { setError(null); setIsLoading(false); }}
              className="ml-2 underline"
            >
              Dismiss
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
