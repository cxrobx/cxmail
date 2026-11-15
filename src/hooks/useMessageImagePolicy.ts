import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import type { ImagePolicy } from "@/types/email";

export const IMAGE_TRUST_CHANGED = "cxmail:image-trust-changed";
const pending = new Map<string, Promise<ImagePolicy>>();

/** Share in-flight header/body checks, never persist a positive permission. */
export function useMessageImagePolicy(accountId?: string | null, folder?: string | null, uid?: number) {
  const key = accountId && folder && uid ? JSON.stringify([accountId, folder, uid]) : "";
  const [result, setResult] = useState<{ key: string; policy: ImagePolicy | null } | null>(null);
  useEffect(() => {
    if (!key || !accountId || !folder || !uid) return;
    let active = true;
    let generation = 0;
    const load = () => {
      const token = ++generation;
      setResult({ key, policy: null });
      let request = pending.get(key);
      if (!request) {
        request = Promise.resolve().then(() => api.imageTrust.policy(accountId, folder, uid));
        pending.set(key, request);
        const clear = () => { if (pending.get(key) === request) pending.delete(key); };
        void request.then(clear, clear);
      }
      void request.then((policy) => {
        if (active && token === generation && policy?.authentication) setResult({ key, policy });
      }).catch((error) => {
        void api.system.logClientError("image-policy", String(error));
      });
    };
    const refresh = () => {
      // Never reuse a decision that began before a trust change or focus refresh.
      pending.delete(key);
      load();
    };
    load();
    window.addEventListener("focus", refresh);
    window.addEventListener(IMAGE_TRUST_CHANGED, refresh);
    return () => {
      active = false;
      window.removeEventListener("focus", refresh);
      window.removeEventListener(IMAGE_TRUST_CHANGED, refresh);
    };
  }, [key, accountId, folder, uid]);
  return result?.key === key ? result.policy : null;
}
