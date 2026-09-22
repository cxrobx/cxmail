import {
  AlertTriangle,
  CheckCircle2,
  HelpCircle,
  MailCheck,
  type LucideIcon,
} from "lucide-react";
import type { AttendeeDelivery, AttendeeDeliveryState } from "@/types/email";

/**
 * How each delivery state is presented.
 *
 * The states themselves are resolved in Rust (`db::invite_notifications::derive`)
 * and this module only decides what they *look* like — it must never infer a
 * state, because a second derivation is how two surfaces end up disagreeing
 * (gotcha #36's one-matcher rule).
 *
 * **The wording is load-bearing.** `sent` means Google accepted our request to
 * notify the address; it is not a delivery receipt and says nothing about
 * bounces, spam filing, or tenant quarantine. Calling it "received" would
 * recreate the exact failure this feature exists to fix — `responseStatus`
 * already reads `needsAction` whether someone was invited or was never told,
 * and a confident-sounding label over an unknown is worse than no label. Only
 * `responded` is proof of receipt, because a reply cannot exist without an
 * invitation. Pinned by `attendeeDelivery.test.ts`.
 */
export interface DeliveryPresentation {
  Icon: LucideIcon;
  className: string;
  /** Terse right-aligned status shown in the guest row. */
  summary: string;
  /** Full explanation, used as the row tooltip and screen-reader text. */
  label: string;
}

export const DELIVERY_PRESENTATION: Record<
  AttendeeDeliveryState,
  DeliveryPresentation
> = {
  responded: {
    Icon: CheckCircle2,
    className: "text-success",
    summary: "replied",
    label: "Replied — they definitely received the invitation",
  },
  sent: {
    Icon: MailCheck,
    className: "text-content-secondary",
    summary: "invite sent",
    label:
      "Invite sent, no reply yet — Google accepted the send, which is not a delivery receipt",
  },
  unsent: {
    Icon: AlertTriangle,
    className: "text-warning",
    summary: "not notified",
    label: "Never notified — this person has not been sent an invitation",
  },
  unknown: {
    Icon: HelpCircle,
    className: "text-content-secondary opacity-60",
    summary: "no record",
    label:
      "No record — created or changed outside CXMail, so we cannot tell either way",
  },
};

/**
 * Google's raw `responseStatus` rendered for humans, or null when it carries no
 * information. `needsAction` deliberately returns null: it is the default value
 * stamped on every attendee at attach time, so displaying it as a status is
 * precisely the false signal this feature replaces.
 */
export function responseDetail(attendee: AttendeeDelivery): string | null {
  switch (attendee.response_status?.toLowerCase()) {
    case "accepted":
      return "accepted";
    case "declined":
      return "declined";
    case "tentative":
      return "maybe";
    default:
      return null;
  }
}

/** How many guests we have positive reason to believe were never told. */
export function countUnsent(attendees: AttendeeDelivery[]): number {
  return attendees.filter((attendee) => attendee.state === "unsent").length;
}
