import { describe, expect, it } from "vitest";
import { rankCommands, scoreCommand } from "@/lib/commandScore";

const item = (label: string, category = "General", keywords?: string) => ({
  label,
  category,
  keywords,
});

describe("scoreCommand field tiers", () => {
  it("ranks prefix > word-boundary > mid-word substring > scattered", () => {
    const prefix = scoreCommand("sync", item("Sync Now"));
    const boundary = scoreCommand("sync", item("Force Sync"));
    const midWord = scoreCommand("sync", item("Resyncing"));
    const scattered = scoreCommand("sync", item("Stay in Contact"));
    expect(prefix).toBeGreaterThan(boundary);
    expect(boundary).toBeGreaterThan(midWord);
    expect(midWord).toBeGreaterThan(scattered);
    expect(scattered).toBeGreaterThan(0);
  });

  it("weights label > keywords > category for the same match", () => {
    const inLabel = scoreCommand("inbox", item("Inbox", "Nav"));
    const inKeywords = scoreCommand("inbox", item("Mail", "Nav", "inbox"));
    const inCategory = scoreCommand("inbox", item("Mail", "Inbox"));
    expect(inLabel).toBeGreaterThan(inKeywords);
    expect(inKeywords).toBeGreaterThan(inCategory);
  });

  it("returns 0 when nothing matches anywhere", () => {
    expect(scoreCommand("zzqx", item("Inbox", "Navigation", "folder"))).toBe(0);
  });
});

describe("recall-guarantee fallback", () => {
  it("still matches queries the old concatenated-haystack filter matched", () => {
    // "boxnav" is not a subsequence of any single field, but IS a subsequence
    // of "Inbox Navigation folder" — the old filter's exact haystack.
    const cmd = item("Inbox", "Navigation", "folder");
    expect(scoreCommand("boxnav", cmd)).toBeGreaterThan(0);
  });

  it("ranks fallback-tier matches below any single-field match", () => {
    const fallback = item("Inbox", "Navigation", "folder"); // matches only via haystack
    const direct = item("Box Navigator", "Other"); // "boxnav" scattered in label
    const ranked = rankCommands("boxnav", [fallback, direct]);
    expect(ranked.map((r) => r.label)).toEqual(["Box Navigator", "Inbox"]);
  });
});

describe("rankCommands", () => {
  it("drops non-matches and orders by score", () => {
    const items = [
      item("Stay in Contact"),
      item("Sync Now"),
      item("Compose"),
      item("Force Sync"),
    ];
    const ranked = rankCommands("sync", items);
    expect(ranked.map((r) => r.label)).toEqual([
      "Sync Now",
      "Force Sync",
      "Stay in Contact",
    ]);
  });

  it("breaks score ties by shorter label, then original index", () => {
    const a = item("Archive");
    const b = item("Archive"); // identical: same score, same length → index
    const shorter = item("Arch");
    const ranked = rankCommands("arch", [a, b, shorter]);
    expect(ranked[0]).toBe(shorter);
    expect(ranked[1]).toBe(a);
    expect(ranked[2]).toBe(b);
  });

  it("returns all items unchanged on an empty query", () => {
    const items = [item("B"), item("A")];
    expect(rankCommands("  ", items).map((r) => r.label)).toEqual(["B", "A"]);
  });
});
