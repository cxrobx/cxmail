import { useCallback, useEffect, useState } from "react";
import { AtSign, Plus, Trash2 } from "lucide-react";
import { api } from "@/lib/tauri";
import { normalizeAddr, sameAddress } from "@/lib/sendAs";
import type { Account, SendAsAddress, SendAsSuggestion } from "@/types/email";
import { cn } from "@/lib/utils";

/**
 * Send-as addresses, in the Settings dialog.
 *
 * An account can own alias addresses (iCloud and Gmail both allow it), and
 * until now CXMail had no way to know about them: compose always sent from the
 * account's own address, so a reply to mail that arrived at an alias went out
 * from the primary with no way to change it.
 *
 * They are USER-ENTERED, and that is a finding rather than a shortcut. IMAP is
 * a mailbox-access protocol with no identity extension, so there is nothing to
 * ask. The provider APIs that do know are outside CXMail's grant: Gmail's
 * `users.settings.sendAs.list` needs `gmail.settings.basic` while CXMail holds
 * plain `https://mail.google.com/` for IMAP/SMTP, and widening it runs into the
 * restricted-scope verification + CASA wall that made generic IMAP worth
 * building in the first place; iCloud exposes aliases only through its private
 * web API. So the compromise is to make typing one cheap: `suggest_send_as`
 * offers the same-domain addresses this mailbox has actually RECEIVED at,
 * most-received first, and the user confirms which of them is really an alias.
 *
 * ⚠ Adding an address here does not make the provider accept it. SMTP still
 * authenticates as the account, and a provider that does not recognise the
 * alias will reject or rewrite the send — which is why the note below says so
 * rather than implying the app can grant the right.
 */
export default function SendAsSection() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [accountId, setAccountId] = useState<string>("");
  const [addresses, setAddresses] = useState<SendAsAddress[]>([]);
  const [suggestions, setSuggestions] = useState<SendAsSuggestion[]>([]);
  const [newEmail, setNewEmail] = useState("");
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    // `Promise.resolve(...)`, as in TriageSection: a backend (or test double)
    // that answers with no promise must not take the Settings dialog down.
    Promise.resolve(api.accounts.list())
      .then((list) => {
        const accounts = Array.isArray(list) ? list : [];
        setAccounts(accounts);
        setAccountId((current) => current || accounts[0]?.id || "");
      })
      .catch((e) => console.error("Failed to load accounts:", e));
  }, []);

  const reload = useCallback(async (id: string) => {
    if (!id) return;
    const [list, hints] = await Promise.all([
      Promise.resolve(api.identities.listSendAs(id)).catch(() => [] as SendAsAddress[]),
      Promise.resolve(api.identities.suggestSendAs(id)).catch(() => [] as SendAsSuggestion[]),
    ]);
    setAddresses(Array.isArray(list) ? list : []);
    setSuggestions(Array.isArray(hints) ? hints : []);
  }, []);

  useEffect(() => {
    void reload(accountId);
  }, [accountId, reload]);

  const account = accounts.find((a) => a.id === accountId);

  const add = useCallback(async () => {
    const email = newEmail.trim();
    if (!email) return;
    // Both checks fail SILENTLY without this. A malformed address is accepted
    // by the store and then refused at the write boundary, much later; and the
    // account's own address is folded into the primary by `build_send_as`, so
    // adding it looks like the button did nothing at all.
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      setError("That does not look like an email address.");
      return;
    }
    if (account && sameAddress(email, account.email)) {
      setError("That is the account's own address — it is always available to send from.");
      return;
    }
    if (addresses.some((a) => sameAddress(a.email, email))) {
      setError("Already configured.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.identities.create({
        id: null,
        account_id: accountId,
        email,
        display_name: newName.trim() || null,
        // Left empty on purpose: an alias with no signature INHERITS the
        // account's rather than sending unsigned, so a blank here is the
        // sensible default rather than a missing setting.
        signature_html: null,
        is_default: false,
      });
      setNewEmail("");
      setNewName("");
      await reload(accountId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [account, accountId, addresses, newEmail, newName, reload]);

  const remove = useCallback(
    async (address: SendAsAddress) => {
      if (address.identity_id == null) return;
      setBusy(true);
      setError(null);
      try {
        await api.identities.delete(address.identity_id);
        await reload(accountId);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [accountId, reload],
  );

  if (accounts.length === 0) return null;

  const unconfigured = suggestions.filter(
    (s) => !addresses.some((a) => normalizeAddr(a.email) === normalizeAddr(s.email)),
  );

  return (
    <section>
      <h3 className="mb-2 text-xs font-medium uppercase tracking-wider text-content-muted">
        Send-as addresses
      </h3>

      {accounts.length > 1 && (
        <select
          aria-label="Account"
          value={accountId}
          onChange={(e) => setAccountId(e.target.value)}
          className="mb-2 w-full rounded-md border border-border bg-input px-2 py-1.5 text-xs text-content"
        >
          {accounts.map((a) => (
            <option key={a.id} value={a.id}>
              {a.email}
            </option>
          ))}
        </select>
      )}

      <ul className="space-y-1">
        {addresses.map((a) => (
          <li
            key={`${a.account_id}:${a.email}`}
            className="flex items-center gap-2 rounded-md bg-surface px-2 py-1.5 text-xs"
          >
            <AtSign className="h-3 w-3 shrink-0 text-content-faint" />
            <span className="min-w-0 flex-1 truncate text-content">{a.email}</span>
            {a.is_primary ? (
              // The account's own address is not a row anyone created and
              // cannot be removed — deleting the identity row that folds into
              // it would only drop its signature, which is not what a bin icon
              // next to an address reads as.
              <span className="shrink-0 text-[10px] uppercase tracking-wide text-content-faint">
                primary
              </span>
            ) : (
              <button
                onClick={() => remove(a)}
                disabled={busy}
                aria-label={`Remove ${a.email}`}
                className="shrink-0 rounded p-0.5 text-content-faint hover:text-content"
              >
                <Trash2 className="h-3 w-3" />
              </button>
            )}
          </li>
        ))}
      </ul>

      <div className="mt-2 flex gap-1">
        <input
          type="email"
          value={newEmail}
          onChange={(e) => {
            setNewEmail(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") void add();
          }}
          placeholder="alias@example.com"
          aria-label="New send-as address"
          className="min-w-0 flex-1 rounded-md border border-border bg-input px-2 py-1.5 text-xs text-content placeholder-content-faint"
        />
        <input
          type="text"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void add();
          }}
          placeholder="Name (optional)"
          aria-label="Display name for the new address"
          className="w-28 shrink-0 rounded-md border border-border bg-input px-2 py-1.5 text-xs text-content placeholder-content-faint"
        />
        <button
          onClick={() => void add()}
          disabled={busy || !newEmail.trim()}
          aria-label="Add send-as address"
          className={cn(
            "shrink-0 rounded-md border border-border px-2 text-content-secondary hover:text-content",
            (busy || !newEmail.trim()) && "opacity-40",
          )}
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
      </div>

      {error && <p className="mt-1.5 text-[11px] text-error">{error}</p>}

      {unconfigured.length > 0 && (
        <div className="mt-2">
          <p className="mb-1 text-[11px] text-content-faint">
            Addresses on this domain that mail has arrived at:
          </p>
          <div className="flex flex-wrap gap-1">
            {unconfigured.map((s) => (
              <button
                key={s.email}
                onClick={() => {
                  setNewEmail(s.email);
                  setError(null);
                }}
                className="rounded-full border border-border px-2 py-0.5 text-[11px] text-content-secondary hover:text-content"
              >
                {s.email}
                <span className="ml-1 text-content-faint">{s.message_count}</span>
              </button>
            ))}
          </div>
        </div>
      )}

      <p className="mt-2.5 text-[11px] leading-snug text-content-faint">
        Replies default to the address the original was sent to. Sending still signs in as the
        account itself, so the provider has to recognise the alias too — adding it here does not
        grant the right, it only lets CXMail ask.
      </p>
    </section>
  );
}
