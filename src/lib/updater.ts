import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useUIStore } from "@/stores/uiStore";
import { api } from "@/lib/tauri";

/** `owner_bypass_status()` in commands/license.rs reports this as the masked key
 *  for a build compiled with CXMAIL_OWNER_LICENSE. A customer build never can:
 *  a real key is masked from its own digits, and an unlicensed build reports null. */
const OWNER_BUILD_MASKED_KEY = "OWNER";

export async function checkForAppUpdate(): Promise<void> {
  if (!import.meta.env.PROD) return;

  // An owner build must never replace itself with a published artifact.
  // Published builds carry no owner bypass, so installing one over this build
  // would drop the user behind the "Activate CXMail" gate with no key — locking
  // the maintainer out of their own mail with one click on an update toast.
  // Owner builds are maintained from source by whoever compiled them, so there
  // is no case where accepting a published update is the right outcome.
  try {
    if ((await api.license.getStatus()).maskedKey === OWNER_BUILD_MASKED_KEY) {
      void api.system.logClientError("updater", "owner build — update check skipped");
      return;
    }
  } catch (error) {
    // Never let this guard become a reason updates stop working for customers:
    // if the status read fails, fall through to the normal check.
    void api.system.logClientError("updater", `license status unreadable: ${String(error)}`);
  }

  try {
    const update = await check({ timeout: 15_000 });
    if (!update) return;
    useUIStore.getState().addToast({
      message: `CXMail ${update.version} is ready to install`,
      type: "info",
      duration: 30_000,
      action: {
        label: "Update",
        onClick: () => {
          void update.downloadAndInstall().then(() => relaunch()).catch((error) => {
            // The toast is transient and the user is unlikely to transcribe it;
            // a failed install is exactly what a bug report needs to carry.
            void api.system.logClientError("updater-install", String(error));
            useUIStore.getState().addToast({
              message: `Update failed: ${String(error)}`,
              type: "error",
            });
          });
        },
      },
    });
  } catch (error) {
    // Must reach the Rust log, not just the console: this function only runs
    // when PROD, and release builds ship without DevTools (gotcha #19), so a
    // console.warn here is unreadable by anyone. A dead update endpoint went
    // unnoticed for weeks behind exactly this line — the only reason it
    // surfaced at all is that tauri-plugin-updater logs its own error.
    console.warn("Update check failed:", error);
    void api.system.logClientError("updater", String(error));
  }
}
