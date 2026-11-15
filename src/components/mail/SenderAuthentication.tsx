import { BadgeCheck, CircleHelp, ShieldAlert } from "lucide-react";
import { useMessageImagePolicy } from "@/hooks/useMessageImagePolicy";

export default function SenderAuthentication({ accountId, folder, uid }: { accountId: string | null; folder: string | null; uid: number }) {
  const policy = useMessageImagePolicy(accountId, folder, uid);
  const auth = policy?.authentication;
  const Icon = auth?.status === "passed" ? BadgeCheck : auth?.status === "failed" ? ShieldAlert : CircleHelp;
  const label = auth?.status === "passed" ? `Domain authenticated: ${auth.domain}` : auth?.status === "failed" ? "Domain authentication failed" : "Sender authentication unavailable";
  return <span className="ml-1 inline-flex align-middle text-content-muted" title={`${label}. ${auth?.explanation ?? "Checking authentication; automatic images remain blocked."}`} role="img" aria-label={label}>
    <Icon className="h-3.5 w-3.5" />
  </span>;
}
