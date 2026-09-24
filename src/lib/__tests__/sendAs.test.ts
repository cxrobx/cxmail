import { describe, it, expect } from "vitest";
import {
  fromOptions,
  normalizeAddr,
  primaryOnly,
  resolveFrom,
  sameAddress,
  sendAsLabel,
  shouldShowFromPicker,
} from "@/lib/sendAs";
import type { Account, SendAsAddress } from "@/types/email";

function account(id: string, email: string, name: string | null = null): Account {
  return { id, email, display_name: name } as Account;
}

function address(
  accountId: string,
  email: string,
  isPrimary: boolean,
  name: string | null = null,
): SendAsAddress {
  return {
    account_id: accountId,
    email,
    display_name: name,
    signature_html: null,
    is_primary: isPrimary,
    identity_id: isPrimary ? null : 1,
  };
}

const ICLOUD = account("ic", "sidalias@icloud.com", "iCloud");
const CXV = account("cxv", "chris@cxventures.io", "CX Ventures");
const ICLOUD_ADDRESSES = [
  address("ic", "sidalias@icloud.com", true, "iCloud"),
  address("ic", "rileyprime@icloud.com", false, "Christopher"),
];

describe("normalizeAddr / sameAddress", () => {
  it("compares case- and whitespace-insensitively", () => {
    expect(normalizeAddr("  SidAlias@iCloud.com ")).toBe("sidalias@icloud.com");
    expect(sameAddress("Alias@Icloud.com", "alias@icloud.com")).toBe(true);
  });

  it("treats a missing address as matching nothing — including another missing one", () => {
    // Two composers with no From resolved yet must not read as "the same From".
    expect(sameAddress(null, null)).toBe(false);
    expect(sameAddress("", "")).toBe(false);
    expect(sameAddress(undefined, "a@b.com")).toBe(false);
  });
});

describe("fromOptions", () => {
  it("lists every account's addresses, primary ahead of its own aliases", () => {
    const options = fromOptions([ICLOUD, CXV], {
      ic: ICLOUD_ADDRESSES,
      cxv: [address("cxv", "chris@cxventures.io", true)],
    });
    expect(options.map((o) => o.email)).toEqual([
      "sidalias@icloud.com",
      "rileyprime@icloud.com",
      "chris@cxventures.io",
    ]);
  });

  /// The send-as lists load asynchronously, so an account with no entry yet has
  /// to contribute its own address. Dropping it makes accounts flicker out of
  /// the From menu on every open.
  it("falls back to the account's own address while its list is still loading", () => {
    expect(fromOptions([ICLOUD, CXV], {}).map((o) => o.email)).toEqual([
      "sidalias@icloud.com",
      "chris@cxventures.io",
    ]);
    expect(fromOptions([ICLOUD], { ic: [] })[0].is_primary).toBe(true);
  });
});

describe("resolveFrom", () => {
  const options = fromOptions([ICLOUD, CXV], {
    ic: ICLOUD_ADDRESSES,
    cxv: [address("cxv", "chris@cxventures.io", true)],
  });

  it("defaults to the account's own address", () => {
    expect(resolveFrom(options, "ic", null, null)?.email).toBe("sidalias@icloud.com");
  });

  /// The whole point of the feature, at the UI layer: a reply to mail that
  /// arrived at the alias opens addressed FROM the alias.
  it("uses the reply default when the user has not picked", () => {
    const got = resolveFrom(options, "ic", null, "rileyprime@icloud.com");
    expect(got?.email).toBe("rileyprime@icloud.com");
    expect(got?.is_primary).toBe(false);
  });

  /// `reply_from_for_message` is a round trip and can land after the user has
  /// already opened the menu and chosen. A person's choice outranks a default,
  /// always — which is why the two are separate rungs rather than one piece of
  /// seeded state.
  it("never lets a late reply default overwrite the user's own pick", () => {
    expect(resolveFrom(options, "ic", "sidalias@icloud.com", "rileyprime@icloud.com")?.email).toBe(
      "sidalias@icloud.com",
    );
  });

  /// Switching the From account leaves the previous account's address
  /// selected. Carrying it would put an address on the wire that the new
  /// account cannot send as — refused by the backend, at send time, after the
  /// user thought they were done.
  it("ignores an override belonging to a different account and falls back visibly", () => {
    const got = resolveFrom(options, "cxv", "rileyprime@icloud.com", null);
    expect(got?.email).toBe("chris@cxventures.io");
  });

  it("ignores a reply default belonging to a different account", () => {
    expect(resolveFrom(options, "cxv", null, "rileyprime@icloud.com")?.email).toBe(
      "chris@cxventures.io",
    );
  });

  it("matches the override case-insensitively", () => {
    expect(resolveFrom(options, "ic", "RileyPrime@ICLOUD.com", null)?.email).toBe(
      "rileyprime@icloud.com",
    );
  });

  it("is undefined only when there is nothing to choose from", () => {
    expect(resolveFrom([], "ic", null, null)).toBeUndefined();
  });

  /// An address that is no longer configured (the user removed the alias while
  /// the window was open) must not be sent from — the whole list would
  /// otherwise be bypassed by a stale string.
  it("drops an override that is no longer a configured address", () => {
    expect(resolveFrom(options, "ic", "deleted@icloud.com", null)?.email).toBe(
      "sidalias@icloud.com",
    );
  });
});

describe("the picker's chrome", () => {
  it("stays hidden for the single-account, single-address case", () => {
    expect(shouldShowFromPicker(primaryOnly(ICLOUD))).toBe(false);
    expect(shouldShowFromPicker(ICLOUD_ADDRESSES)).toBe(true);
  });

  it("labels an address with its name when it has one", () => {
    expect(sendAsLabel(ICLOUD_ADDRESSES[1])).toBe("Christopher <rileyprime@icloud.com>");
    expect(sendAsLabel(address("ic", "bare@icloud.com", false))).toBe("bare@icloud.com");
    expect(sendAsLabel(undefined)).toBe("");
  });
});
