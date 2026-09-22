import { useEffect, useState } from "react";
import { api, type LicenseStatus } from "@/lib/tauri";
import { KeyRound, Loader2 } from "lucide-react";

export default function LicenseGate({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = useState<LicenseStatus | null>(null);
  const [licenseKey, setLicenseKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.license.refresh().then(setStatus).catch((cause) => {
      setError(String(cause));
      api.license.getStatus().then(setStatus).catch(() => setStatus({
        active: false, maskedKey: null, validatedAt: null, offlineGrace: false, developmentBuild: false,
      }));
    });
  }, []);

  const activate = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.license.activate(licenseKey));
      setLicenseKey("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  if (!status) {
    return <div className="flex h-screen items-center justify-center bg-base"><Loader2 className="h-6 w-6 animate-spin text-accent" /></div>;
  }
  if (status.active) return <>{children}</>;

  return (
    <div className="flex h-screen items-center justify-center bg-base px-6">
      <div className="w-full max-w-md text-center">
        <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl bg-accent/10">
          <KeyRound className="h-8 w-8 text-accent" />
        </div>
        <h1 className="mt-6 text-2xl font-semibold text-content">Activate CXMail</h1>
        <p className="mt-2 text-sm leading-relaxed text-content-secondary">
          Enter the license key from your CX Ventures receipt. Your license includes perpetual use of CXMail and 12 months of feature updates.
        </p>
        <input
          value={licenseKey}
          onChange={(event) => setLicenseKey(event.target.value.toUpperCase())}
          onKeyDown={(event) => event.key === "Enter" && void activate()}
          placeholder="CXM-XXXXXXXX-XXXXXXXX-XXXXXXXXXX-XXXXXXXXXX"
          className="mt-8 w-full rounded-lg border border-border bg-surface px-4 py-3 text-center font-mono text-sm text-content outline-none focus:border-accent"
        />
        <button
          onClick={() => void activate()}
          disabled={busy || !licenseKey.trim()}
          className="mt-3 flex w-full items-center justify-center gap-2 rounded-lg bg-accent px-4 py-3 text-sm font-medium text-white hover:bg-accent-hover disabled:opacity-50"
        >
          {busy && <Loader2 className="h-4 w-4 animate-spin" />}
          {busy ? "Activating…" : "Activate license"}
        </button>
        {error && <p className="mt-3 text-sm text-red-400">{error}</p>}
        <a href="https://cxventures.io/products/cxmail" className="mt-6 inline-block text-sm text-content-muted hover:text-content">Buy a CXMail license →</a>
      </div>
    </div>
  );
}
