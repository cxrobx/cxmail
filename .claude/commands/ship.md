---
description: Build the CXMail app (and rebuild the live MCP binary); ASK before swapping the running app
---

# Ship CXMail

Rebuild the Tauri app bundle. Standing rule: always rebuild before relaunching after code changes.

Working directory: the repository root

> ⚠️ **CXMail is Chris's live email client.** NEVER quit or replace the running
> `/Applications/CXMail.app` on your own. Build the bundle, then **stop and ask Chris** before the
> swap step. He may want to finish what he's doing first.

> ⚠️ **Rule #8 — live MCP binary.** `src-tauri/target/release/cxmail-mcp` is spawned by an active
> Claude MCP server registration (in `~/.claude.json`, command =
> `…/cxmail/src-tauri/target/release/cxmail-mcp`). Never `cargo clean` here without an immediate
> rebuild. Note: the Rule #8 doc grep targets `~/.mcp.json`, but this binary is actually registered
> in `~/.claude.json`, so that grep won't catch it.

## Steps

Always `source "$HOME/.cargo/env"` before any cargo/tauri command (gotcha #1).

**Use the project's Node**, not the shell default — a Bash session starts on Node 20 and this
repo needs 24 (gotcha #42). The frontend build runs inside `npm run tauri build`:
```bash
export PATH="$HOME/.nvm/versions/node/v24.11.1/bin:$PATH"
```

0. **Stage a helper that carries the Google OAuth client.** The Google client id and
   secret are not in the source tree (`secrets.rs` header), and `npm run tauri build`
   bundles `src-tauri/binaries/cxmail-helper-aarch64-apple-darwin` exactly as committed
   — and the committed file is a placeholder shell script that only prints an error.
   So build a real one over it now, and put the placeholder back in step 1b:
   ```bash
   source "$HOME/.cargo/env" && cd src-tauri && \
     secret run -k CXMAIL_GOOGLE_CLIENT_ID -k CXMAIL_GOOGLE_CLIENT_SECRET -- \
     cargo build --release --bin cxmail-helper --target aarch64-apple-darwin && \
     cp target/aarch64-apple-darwin/release/cxmail-helper binaries/cxmail-helper-aarch64-apple-darwin && cd ..
   ```

1. **Build the app bundle** → `src-tauri/target/release/bundle/macos/CXMail.app`
   (`productName` became `CXMail` in the 1.0 commit; the lowercase path still resolves
   because the volume is case-insensitive):
   ```bash
   source "$HOME/.cargo/env" && CXMAIL_OWNER_LICENSE=owner TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
     secret run -k TAURI_SIGNING_PRIVATE_KEY -k CXMAIL_GOOGLE_CLIENT_ID -k CXMAIL_GOOGLE_CLIENT_SECRET \
     -- npm run tauri build
   ```
   The two `CXMAIL_GOOGLE_*` keys are compiled in by `option_env!`; leave them out and
   the build succeeds with **no Gmail sign-in** — `require_client_id` reports "not
   configured" only when someone tries to add or refresh a Gmail account.

   **Do not drop the leading `source`** — it does not just look redundant, it is
   load-bearing here. `secret run` execs its command in a subprocess that does not
   inherit a login shell, so without it cargo is off PATH and the build fails at
   `failed to run 'cargo metadata' … No such file or directory (os error 2)` — an
   error that names cargo in a command line that never mentions cargo.

   Two env vars, both required, for different reasons:
   - `CXMAIL_OWNER_LICENSE=owner` — **only for Chris's own copy.** Without it a release
     build gates him behind "Activate CXMail" (gotcha #28). NEVER set it for customer
     artifacts; the CI release workflow must bake in `None`.
   - `TAURI_SIGNING_PRIVATE_KEY` — `createUpdaterArtifacts` is on, so without the key the
     build errors at the very end ("A public key has been found, but no private key").
     The `.app` is still produced and usable, but no `.sig` is generated, so the build
     cannot be published. Key lives in the login Keychain via secrets-kit; verify with
     `secret has TAURI_SIGNING_PRIVATE_KEY`.
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""` — **required, even though it is empty.**
     rsign2 always encrypts the key (`untrusted comment: rsign encrypted secret key`);
     generating without a password just means the password IS the empty string. Omit
     this var and Tauri tries to prompt on stdin, which in a non-interactive build
     fails as `Device not configured (os error 6)` — an error that reads like a
     hardware problem and has nothing to do with one.

1a. **Confirm the bundle carries the real helper**, not the committed placeholder script
   (a skipped step 0 builds fine and ships a helper that only prints an error):
   ```bash
   file src-tauri/target/release/bundle/macos/CXMail.app/Contents/MacOS/cxmail-helper | grep -q Mach-O \
     && echo "helper OK" || echo "STOP: bundled helper is the placeholder — redo step 0"
   ```

1b. **Put the committed placeholder back**, so the staged copy (which holds the secret) is
   never committed: `git checkout -- src-tauri/binaries/cxmail-helper-aarch64-apple-darwin`,
   then `git status --short` must be empty.

   Checking a binary for the secret: **use Python, not `grep`**. Under `sh -c` (which is
   how `secret run` runs a check) `grep -a -c -F` returned 0 on a 7 MB helper that
   contained the secret; `python3 -c 'import os,sys; print(os.environ["S"].encode() in open(sys.argv[1],"rb").read())'`
   is the check that held up.
2. **If you changed `src-tauri/` MCP source, rebuild the live MCP binary IMMEDIATELY** (Rule #8 — the
   registered MCP server points at this exact path; leaving it stale/absent breaks the `cxmail` MCP):
   ```bash
   source "$HOME/.cargo/env" && cd src-tauri && \
     secret run -k CXMAIL_GOOGLE_CLIENT_ID -k CXMAIL_GOOGLE_CLIENT_SECRET -- \
     cargo build --release --bin cxmail-mcp && cd ..
   strings src-tauri/target/release/cxmail-mcp | grep -q apps.googleusercontent.com \
     && echo "client id OK" || echo "STOP: MCP built without the Google client — rebuild with secret run"
   ```
   **The `secret run` is not optional.** The Google client is compiled in by `option_env!`, so a
   bare `cargo build` succeeds and produces an MCP whose every Gmail token refresh sends
   `client_id=""` — Google answers *"Could not determine client ID from request"*, which reads
   like a broken account and sends you off to reconnect it. Happened 2026-09-23 (T207 ship):
   the draft for chris@artistadvisory.io failed minutes after this step ran bare. The check line
   above is the tripwire; the ID is public, so `strings` is fine for it (the *secret* needs the
   Python check in 1b).
   Restart the `cxmail` MCP session afterward so the new binary is loaded.

   > **No `-p`, and that is deliberate.** `src-tauri/` is a Cargo workspace now, so a
   > `-p` flag looks like the missing piece. It is not: `[[bin]] cxmail-mcp` belongs to the
   > **app package**, not to the `cxmail-mcp` crate (which is lib-only). `-p cxmail-mcp` would
   > select the library and find no such bin target. Adding one to that crate would collide
   > with this one in the shared `target/release/`. Leave the command exactly as written.
   >
   > The bins stay in the app package because Tauri's bundler enumerates *that* package's
   > `[[bin]]` targets — `tauri.conf.json` declares only `binaries/cxmail-helper`, so
   > `cxmail-mcp` and `cxmail-tracker` reach `cxmail.app/Contents/MacOS/` by no other route.
   > A binary that stops being bundled is one `keychain::macos_acl` silently skips when it
   > builds the shared-ACL trusted-binary list (`if p.exists()`), and the reward is a Keychain
   > prompt on every credential read, forever — the exact failure gotcha #31 exists to prevent.

   > **Step 1 already rebuilt this binary for you — which is exactly the problem.** The bundle
   > ships `cxmail-mcp`, so `npm run tauri build` re-links
   > `src-tauri/target/release/cxmail-mcp` whether or not you touched MCP source, and a re-link
   > **destroys its signature**. Tauri then signs the *copy inside the bundle* and leaves the loose
   > one ad-hoc. So this step is usually a no-op and **step 2b never is.**

2b. **Codesign the loose MCP binary — UNCONDITIONALLY, on every ship, after step 1.** This is not
   conditional on step 2 or on having touched MCP source: step 1 re-links the binary by itself, so
   a ship that skips 2b leaves `src-tauri/target/release/cxmail-mcp` ad-hoc even when no Rust
   changed. Observed live on 2026-08-10 — a binary signed correctly earlier in the session came out
   of `npm run tauri build` as `Signature=adhoc`, `TeamIdentifier=not set`. **Check, don't assume:**
   ```bash
   codesign -dv --verbose=2 src-tauri/target/release/cxmail-mcp 2>&1 | grep -E "Signature|TeamIdentifier"
   ```
   Cargo emits an ad-hoc (`linker-signed`) binary with no team identity. Since the Jul 29 cross-store fallback
   (`4097506`), `cxmail-mcp` reads the macOS Keychain for any credential missing from
   `credentials.dat` — `gmail:{email}:calendar:*` and `ai:{provider,model,base_url}` are
   Keychain-only — and an ad-hoc binary can never satisfy the `teamid:CCYV5HQZCM` partition list
   those items carry. Result: a "CXMail wants to use your confidential information" prompt on every
   calendar / voice-profile MCP call, forever ("Always Allow" only whitelists that one binary hash,
   which the next rebuild invalidates). Signing with the Developer ID makes it match the partition
   list like the bundled app does.
   ```bash
   cd src-tauri/target/release
   cp cxmail-mcp cxmail-mcp.signing
   codesign --force --timestamp \
     --sign "Developer ID Application: Christopher Robinson (CCYV5HQZCM)" cxmail-mcp.signing
   mv -f cxmail-mcp.signing cxmail-mcp    # rename, so live MCP processes keep their old inode
   cd ../../..
   codesign -dv --verbose=2 src-tauri/target/release/cxmail-mcp 2>&1 | grep TeamIdentifier
   ```
   Expect `TeamIdentifier=CCYV5HQZCM` (ad-hoc shows `not set`). Sign a **copy and rename** rather than
   in place — other Claude sessions may be running this exact binary, and rename leaves them on the
   old inode instead of invalidating a mapped signature. `--timestamp` needs network; drop it offline.

   Then confirm it satisfies the *installed* app's designated requirement — a valid signature with
   the wrong `identifier` still prompts, and looks identical to a working one until it does:
   ```bash
   REQ=$(codesign -d -r- /Applications/CXMail.app/Contents/MacOS/cxmail-mcp | sed -n 's/^designated => //p')
   codesign --verify -R="$REQ" src-tauri/target/release/cxmail-mcp && echo MATCH
   ```

3. **STOP. Ask Chris before touching the running app.** Do not proceed to the swap without an explicit
   "yes." Report that the build succeeded and the bundle is at
   `src-tauri/target/release/bundle/macos/cxmail.app`, awaiting his go-ahead.

3b. **Check for unsaved compose windows before the swap — "it autosaves" is not true when it
   matters.** Step 4 quits the app, and a draft that has not successfully APPENDed to IMAP exists
   *only* in that window's memory. `commands/compose.rs::save_draft` opens IMAP on its first line
   and returns on `?`, so a failed connect skips **both** the Gmail APPEND and the local-cache
   write on line 281 — nothing is persisted anywhere, and the compose window shows a red
   "Failed to save draft" bar that is easy to miss under a long body.

   This is not hypothetical: on 2026-08-14 the saved copy of an in-flight client email was 20
   minutes stale, still carrying a `[ paste password here ]` placeholder and two paragraphs Chris
   had since deleted. Quitting would have silently reverted the work.

   Compare each recent draft against what is actually in the window before asking for the swap:
   ```bash
   sqlite3 ~/Library/Application\ Support/com.cxmail.app/cxmail.db "
     SELECT m.uid, m.subject, datetime(m.date) AS last_saved
     FROM messages m JOIN accounts a ON a.id=m.account_id
     WHERE m.folder_name LIKE '%Drafts%' AND m.date > datetime('now','-1 day')
     ORDER BY m.date DESC;"
   ```
   A `last_saved` noticeably older than the editing session means unsaved work. Ask Chris to send
   it, or to click save until one succeeds; if IMAP is failing, **copy the body and recipients to
   the scratchpad first** and recreate the draft via `mcp__cxmail__compose_draft` after relaunch.
   Recreating is safe — omit `layout` for ordinary prose (gotcha #34).

4. **Only after Chris confirms** — stop the helper, quit, replace, relaunch, restart the helper:
   ```bash
   LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister
   launchctl bootout gui/$(id -u)/com.cxmail.app.helper 2>/dev/null || true
   osascript -e 'if application "cxmail" is running then quit application "cxmail"' 2>/dev/null || true
   sleep 2
   rm -rf "/Applications/CXMail.app"
   cp -R "src-tauri/target/release/bundle/macos/CXMail.app" /Applications/
   "$LSREGISTER" -f -R "/Applications/CXMail.app"   # or the Dock icon breaks — see below
   open "/Applications/CXMail.app"    # by PATH — see below
   launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.cxmail.app.helper.plist 2>/dev/null || true
   killall Dock                                     # picks up the re-registered bundle
   ```

   **`CXMail.app`, with the capitals.** `productName` is `"CXMail"`, so that is what the
   bundler emits and what sits in `/Applications`. The lowercase spelling this step used
   until 2026-08-31 resolved only because the volume is case-insensitive — and it is a live
   trap, because `cp -R cxmail.app /Applications/` with `CXMail.app` already present copies
   *into* the existing bundle (`/Applications/CXMail.app/CXMail.app`) rather than replacing
   it. The `rm -rf` is the only reason that never fired.

   **The `lsregister` + `killall Dock` lines are why the Dock icon stops turning into a
   "?".** `rm -rf` frees the bundle's directory inode and `cp -R` allocates a new one, so
   every ship hands the app a new identity on disk. The Dock pins its tile with a CFURL
   bookmark that stores the **inode (CNID)**, not just the path, and Launch Services keys
   its record the same way — so both go dead at the swap, and the tile falls back to the
   generic "?" placeholder while still showing the label "CXMail" (that string is
   `file-label` in the Dock's own plist, which is why the name survives and the art does
   not). Restarting the Dock alone does **not** repair it: measured 2026-08-31, the
   bookmark still held CNID 602046074 against a live inode of 603699637 after a `killall
   Dock`. Re-registering the bundle is what fixes it; the Dock restart just makes it repaint.

   Stale copies of the bundle compound it — each keeps its own Launch Services `path:`
   record and its own `mailto` claim, and competes to answer for the bundle id. On
   2026-08-31 there were two `~/Archives/cxmail-app-backup-*.app` and one
   `/Applications/cxmail.app.rollback.*`, all 1.0.0, none notarized; retiring and deleting
   them took the `mailto` claims from four to two. Unregister a copy you want to KEEP on
   disk rather than deleting it — `"$LSREGISTER" -u <path>` touches no files.

   **Count the claims, not the `bundle id:` lines.** `lsregister -dump | grep '^bundle id:
   .*com\.cxmail\.app'` reads **four on a perfectly clean system** — those are the app's
   container directories (`~/Library/{WebKit,Application Support,Logs,Caches}/com.cxmail.app`),
   not app registrations, and they belong there. The honest signals are:
   ```bash
   "$LSREGISTER" -dump | grep -E "^claim id:.*com\.cxmail\.app" | sort -u   # expect 2
   "$LSREGISTER" -dump | grep -iE "^path:.*/cxmail\.app( \(0x[0-9a-f]+\))?$" | sort -u   # expect 2
   ```
   Anchor that path grep on `/cxmail.app$`. A loose `grep "\.app"` reads **six**, because
   the container directories are literally named `com.cxmail.app` and so end in `.app` too —
   the same four that inflate the `bundle id:` count, counted a second way.

   Two, not one, is correct and healthy: `/Applications/CXMail.app` plus the build output at
   `src-tauri/target/release/bundle/macos/CXMail.app`. That second one is the very bundle the
   `open`-by-path rule above exists to avoid launching — it is a build artifact, so do not
   delete it; it returns on the next build.

   **A third, `/Volumes/dmg.XXXXXX/CXMail.app`, is the build's own leftover** — `bundle_dmg.sh`
   mounts a scratch image to lay out the `.dmg`, Launch Services registers the app inside it,
   and the record outlives the unmount (seen 2026-09-21: 3 claims, 3 paths). Confirm the volume
   is gone (`ls -d /Volumes/dmg.*`), then drop the record with
   `"$LSREGISTER" -u /Volumes/dmg.XXXXXX/CXMail.app` — it touches no files — and re-count.

   **`open` by path, never `open -a cxmail`.** Step 1 just registered a second
   `CXMail.app` with LaunchServices — the one in
   `src-tauri/target/release/bundle/macos/` — and `open -a` by name picked *that*
   one on 2026-08-24: the app came up running from the build directory (visible
   in `ps` as `…/bundle/macos/CXMail.app/Contents/MacOS/cxmail`) while the helper
   ran from `/Applications`. Same bits, wrong path: the next `cargo build` re-links
   the binary under a running process.

   **Check that with `pgrep -fi`, case-insensitively** — `pgrep -f` matches the literal
   argv string, and the two live processes disagree on case: the app runs as
   `/Applications/CXMail.app/Contents/MacOS/cxmail` (from `open` by the real path) while
   the helper runs as `/Applications/cxmail.app/Contents/MacOS/cxmail-helper` (the launchd
   plist hardcodes lowercase). A case-sensitive `pgrep -f "/Applications/cxmail.app/…"`
   therefore never matches the app at all, and reports the exact "no app after the swap"
   failure it is supposed to detect — every time, even on a healthy ship:
   ```bash
   pgrep -fil "/Applications/CXMail.app/Contents/MacOS/cxmail$"   # expect one PID
   ```

   **The two `launchctl` lines are not optional.** `com.cxmail.app.helper` runs
   `/Applications/cxmail.app/Contents/MacOS/cxmail-helper` — a binary *inside the bundle
   this step deletes*. macOS lets you unlink a running executable, so the old helper keeps
   running the deleted inode, holding the same SQLite file as the freshly installed app:
   two writers, gotcha #12's lock contention, appearing only after a ship.

   Nothing self-heals it. Its `KeepAlive` is `{SuccessfulExit = 0}`, so it only respawns
   if it *exits*, and the app re-bootstraps only when the plist is **absent**
   (`lib.rs` guards on `!launchd::is_installed()`) — `bootout` does not delete the plist,
   so the guard stays false. Without the explicit `bootstrap`, the helper stays down until
   next login; without the `bootout`, it stays *stale* indefinitely.

   Confirm both halves afterwards — a stale helper looks identical to a healthy one in
   Activity Monitor:
   ```bash
   launchctl list | grep com.cxmail.app.helper    # expect a live PID
   ps -o comm= -p "$(launchctl list | awk '/com.cxmail.app.helper/{print $1}')"
   ```

5. **Verify**:
   ```bash
   pgrep -f "cxmail.app" >/dev/null && echo "app running" || echo "APP NOT RUNNING"
   ```
   For UI debugging use `npm run tauri dev` — production builds ship without DevTools (gotcha #19).

Report: whether the app build succeeded, whether the MCP binary was rebuilt (and MCP restarted), and
— only if Chris approved the swap — whether the reinstalled app is running.
