import { describe, expect, it } from "vitest";
import { fetchDraftReload } from "@/lib/draftReload";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("remote draft reload during typing", () => {
  it("accepts an unchanged editor after a delayed fetch", async () => {
    const network = deferred<string>();
    const reload = fetchDraftReload(() => network.promise, () => "saved body", () => true);
    network.resolve("new server body");
    expect(await reload).toEqual({ kind: "ready", value: "new server body" });
  });

  it("preserves edits made AFTER the clean-editor check but BEFORE the fetch returns", async () => {
    const network = deferred<string>();
    let editor = "saved body";
    const reload = fetchDraftReload(() => network.promise, () => editor, () => true);
    editor = "new typing that has not autosaved";
    network.resolve("new server body");
    expect(await reload).toEqual({ kind: "edited" });
    expect(editor).toBe("new typing that has not autosaved");
  });

  it("lets explicit Reload discard pre-existing edits, but preserves typing AFTER that choice", async () => {
    const network = deferred<string>();
    let editor = "local edits the user chose to discard";
    const reload = fetchDraftReload(() => network.promise, () => editor, () => true);
    editor += " plus newly typed text";
    network.resolve("new server body");
    expect(await reload).toEqual({ kind: "edited" });
  });

  it("never applies an older response that arrives after a newer reload", async () => {
    const first = deferred<string>();
    let current = 1;
    const older = fetchDraftReload(() => first.promise, () => "saved", () => current === 1);
    current = 2;
    const newer = await fetchDraftReload(async () => "newest body", () => "saved", () => current === 2);
    first.resolve("obsolete body");
    expect(newer).toEqual({ kind: "ready", value: "newest body" });
    expect(await older).toEqual({ kind: "superseded" });
  });

  it("reports fetch failure instead of pretending the draft reloaded", async () => {
    await expect(fetchDraftReload(async () => { throw new Error("offline"); }, () => "saved", () => true))
      .rejects.toThrow("offline");
  });
});
