import type { Account, SendAsAddress } from "@/types/email";

/**
 * Choosing the `From:` address for a compose window.
 *
 * CXMail used to have one From per account, so the picker was an account
 * picker and `buildOutgoingEmail` could read `fromAccount.email`. An account
 * can legitimately own more than one address (iCloud and Gmail both do), and
 * mail that arrived at an alias should be answered from that alias — so the
 * unit the composer selects is now an ADDRESS, and the account is whichever
 * one that address belongs to.
 *
 * The resolution lives here, pure, for the reason `draftRecovery` does: the
 * interesting rules are "an override that is no longer available must not
 * stick" and "the reply default must not fight the user", and both are far
 * easier to pin as functions than as a mounted composer. The *matching* half
 * — which of our addresses a message was sent to — is deliberately NOT here:
 * it runs in Rust (`db::identities::match_send_as`) so that the compose window
 * and the MCP cannot pick different addresses for the same reply (gotcha #36).
 */

/** Lowercase + trim. Mirrors `db::identities::normalize_addr`. */
export function normalizeAddr(addr: string | null | undefined): string {
  return (addr ?? "").trim().toLowerCase();
}

export function sameAddress(a: string | null | undefined, b: string | null | undefined): boolean {
  const left = normalizeAddr(a);
  return left !== "" && left === normalizeAddr(b);
}

/**
 * The address list to show for an account when the backend has not answered
 * (or answered with nothing): the account's own address, alone.
 *
 * Not an error case — it is what every account looked like before aliases
 * existed, and it is what a composer that mounts before `list_send_as`
 * resolves has to render. Returning `[]` here instead would blank the From row
 * for a beat on every open.
 */
export function primaryOnly(account: Account): SendAsAddress[] {
  return [
    {
      account_id: account.id,
      email: account.email,
      display_name: account.display_name ?? null,
      signature_html: null,
      is_primary: true,
      identity_id: null,
    },
  ];
}

/**
 * Every address the picker offers, across every account, in account order with
 * each account's primary ahead of its own aliases.
 *
 * An account missing from `byAccount` contributes its primary rather than
 * disappearing: the send-as lists load asynchronously, and an account that
 * vanishes from the From menu for a beat is worse than one that shows only the
 * address it has always had.
 */
export function fromOptions(
  accounts: Account[],
  byAccount: Record<string, SendAsAddress[] | undefined>,
): SendAsAddress[] {
  return accounts.flatMap((a) => {
    const list = byAccount[a.id];
    return list && list.length > 0 ? list : primaryOnly(a);
  });
}

/**
 * The From this composer is actually sending from.
 *
 * Precedence, and each rung is a decision:
 *  1. `overrideEmail` — the user picked it; nothing may override a person.
 *  2. `replyDefaultEmail` — the address the original was addressed to, as
 *     resolved in Rust. It applies only until the user picks, which is why it
 *     is a separate rung and not a seeded piece of state: seeding it would
 *     make a late-arriving reply default overwrite a choice already made.
 *  3. the account's primary.
 *
 * An override or reply default that is not in `options` is IGNORED, not
 * carried: switching the From account leaves the previous account's address
 * selected, and sending from an address the current account cannot send as is
 * refused by the backend — so it has to fall back here, visibly, before the
 * user hits send.
 */
export function resolveFrom(
  options: SendAsAddress[],
  accountId: string | undefined,
  overrideEmail: string | null,
  replyDefaultEmail: string | null,
): SendAsAddress | undefined {
  const forAccount = options.filter((o) => o.account_id === accountId);
  const pool = forAccount.length > 0 ? forAccount : options;
  const pick = (email: string | null) =>
    email ? pool.find((o) => sameAddress(o.email, email)) : undefined;
  return pick(overrideEmail) ?? pick(replyDefaultEmail) ?? pool.find((o) => o.is_primary) ?? pool[0];
}

/** `Name <addr>` when there is a name, the bare address otherwise. */
export function sendAsLabel(address: SendAsAddress | undefined): string {
  if (!address) return "";
  return address.display_name
    ? `${address.display_name} <${address.email}>`
    : address.email;
}

/**
 * Whether the From row is worth showing at all. One account with one address
 * is the pre-alias world and keeps the pre-alias chrome — a picker with a
 * single immovable entry is noise in a window that is already dense.
 */
export function shouldShowFromPicker(options: SendAsAddress[]): boolean {
  return options.length > 1;
}
