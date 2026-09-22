/**
 * Throwaway harness: glassmorphism / depth exploration for CXMail.
 *
 * The shipped app has ZERO backdrop-filter anywhere in `src/` — this compares
 * three positions on adding it. The app frame is rendered over a simulated
 * desktop wallpaper so translucency has something real to pick up, i.e. what
 * `transparent: true` + NSVisualEffectView would actually give us (the
 * prerequisite, `macOSPrivateApi: true`, is already set in tauri.conf.json).
 *
 * Reads the app's real theme + density tokens off <html>, so the density
 * argument is measured against the shipped values, not invented ones.
 *
 * Not shipped — `glass.html` only, dev server only.
 */
import { useEffect, useLayoutEffect, useState, type CSSProperties } from "react";
import {
  Archive,
  Inbox,
  Minus,
  Paperclip,
  PenSquare,
  Search,
  Send,
  Sparkles,
  Star,
  Trash2,
  X,
} from "lucide-react";

type Variant = "A" | "B" | "C";
type Theme = "dark" | "light";
type Density = "comfortable" | "compact" | "ultra-compact";

/* ── Static mock data ───────────────────────────────────────── */

interface MockMessage {
  id: number;
  from: string;
  initials: string;
  subject: string;
  snippet: string;
  time: string;
  unread: boolean;
  flagged: boolean;
  attachment: boolean;
  tag?: string;
}

const BASE: Omit<MockMessage, "id">[] = [
  { from: "Dana Bennett", initials: "DB", subject: "Re: Onboarding videos — recording schedule", snippet: "Sam and I talked it through. If we can get the hosting piece locked by the 12th we can start recording…", time: "9:41 AM", unread: true, flagged: false, attachment: true, tag: "Northwind" },
  { from: "Morgan Park", initials: "MP", subject: "Portal rebuild — phase two scope question", snippet: "Leadership wants reporting split out from the partner directory. Can we quote those separately?", time: "8:12 AM", unread: true, flagged: true, attachment: false, tag: "Bluestone" },
  { from: "Voltworks Electric", initials: "VE", subject: "Chat responder — after-hours routing", snippet: "One more thing on the intake form. If it comes in after 6pm we still want a fast reply, just a different…", time: "Yesterday", unread: false, flagged: false, attachment: false, tag: "Voltworks" },
  { from: "Sam Ellis", initials: "SE", subject: "Search costs — last month", snippet: "The API spend came in under what we modeled. Numbers attached, take a look when you get a minute.", time: "Yesterday", unread: false, flagged: false, attachment: true, tag: "Harborline" },
  { from: "Stripe", initials: "S", subject: "Your payout is on the way", snippet: "A payout of $1,250.00 is expected to arrive at your bank account on Thursday.", time: "Yesterday", unread: false, flagged: false, attachment: false },
  { from: "Lena Ortiz", initials: "LO", subject: "Beta list — who to invite first", snippet: "Pulled the waitlist into a sheet. Roughly a quarter of them look active enough to be worth a first email.", time: "Mon", unread: true, flagged: false, attachment: true },
  { from: "GitHub", initials: "GH", subject: "[example/cxmail] Release 1.0.0 published", snippet: "The release workflow completed. Artifacts were signed, notarized and stapled.", time: "Mon", unread: false, flagged: false, attachment: false },
  { from: "Sarah Whitfield", initials: "SW", subject: "Intro — a mutual friend suggested we talk", snippet: "I was given your name by a mutual contact. Would you be open to a short conversation this week?", time: "Mon", unread: false, flagged: false, attachment: false },
  { from: "Home server", initials: "HS", subject: "Scheduled task completed", snippet: "Task: nightly-backup. Completed successfully at 03:00.", time: "Sun", unread: false, flagged: false, attachment: false },
  { from: "Priya Nair", initials: "PN", subject: "Dashboard mockups — first pass", snippet: "These look sharp. One ask: can the summary view roll up by team rather than by person?", time: "Sun", unread: false, flagged: false, attachment: true, tag: "Bluestone" },
  { from: "Cloudflare", initials: "CF", subject: "Certificate renewed for example.com", snippet: "Your Universal SSL certificate was renewed automatically. No action is required.", time: "Sat", unread: false, flagged: false, attachment: false },
  { from: "Anna Reyes", initials: "AR", subject: "Re: mixing notes on the Tuesday session", snippet: "The low end on the second verse is doing that thing again. Sending you the stems tonight.", time: "Sat", unread: false, flagged: false, attachment: true },
  { from: "Mercury", initials: "M", subject: "Invoice #1042 was paid", snippet: "Northwind Company paid invoice #1042 for $980.00.", time: "Fri", unread: false, flagged: false, attachment: false },
  { from: "Google Calendar", initials: "GC", subject: "Invitation: Voltworks walkthrough @ Thu Aug 21, 2pm", snippet: "You have been invited to the following event. Join with Zoom.", time: "Fri", unread: false, flagged: false, attachment: false },
];

// Deterministic expansion so the list is long enough to actually scroll.
const MESSAGES: MockMessage[] = Array.from({ length: 42 }, (_, i) => ({
  ...BASE[i % BASE.length],
  id: i,
  unread: i < BASE.length ? BASE[i % BASE.length].unread : false,
}));

const FOLDERS = [
  { icon: Inbox, label: "Inbox", count: 12 },
  { icon: Send, label: "Sent", count: 0 },
  { icon: Sparkles, label: "Needs You", count: 6 },
  { icon: Archive, label: "Archive", count: 0 },
  { icon: Trash2, label: "Trash", count: 0 },
];

const PALETTE_ITEMS = [
  "Compose new message",
  "Search all accounts…",
  "Snooze until tomorrow",
  "Open in Claude",
  "Toggle reading pane",
];

/* ── Glass kits ─────────────────────────────────────────────── */

interface Kit {
  id: Variant;
  title: string;
  thesis: string;
  verdict: string;
  rowMode: "flush" | "raised" | "floating";
  titlebar: CSSProperties;
  sidebar: CSSProperties;
  listPane: CSSProperties;
  readingPane: CSSProperties;
  row: CSSProperties;
  rowSelected: CSSProperties;
  floatWin: CSSProperties;
  floatHeader: CSSProperties;
  palette: CSSProperties;
}

const topEdge = (a: number) => "inset 0 1px 0 rgb(255 255 255 / " + a + ")";
const botEdge = (a: number) => "inset 0 -1px 0 rgb(0 0 0 / " + a + ")";
const ring = (a: number) => "inset 0 0 0 1px rgb(255 255 255 / " + a + ")";

function buildKit(variant: Variant, theme: Theme): Kit {
  const d = theme === "dark";
  // Specular highlight has to back off hard in light mode — the light tiers sit
  // within ~7 RGB points of each other and a strong white edge erases them.
  const hi = d ? 0.09 : 0.85;
  const hiSoft = d ? 0.06 : 0.7;
  const lo = d ? 0.35 : 0.06;
  const glassRing = d ? 0.12 : 0.55;

  if (variant === "A") {
    // Chrome-only glass: blur lives on surfaces that float over content.
    const blur = "blur(30px) saturate(180%)";
    return {
      id: "A",
      title: "Chrome glass",
      thesis:
        "Blur only on surfaces that float over something: window vibrancy, title bar, floating window, command palette. The list and reading pane stay fully opaque.",
      verdict:
        "3-4 blur layers, all static. Rows are untouched, so the virtualizer and the density modes are unaffected. This is the shippable one.",
      rowMode: "flush",
      titlebar: {
        background: "rgb(var(--bg-primary) / " + (d ? 0.5 : 0.62) + ")",
        backdropFilter: blur,
        WebkitBackdropFilter: blur,
        boxShadow: topEdge(hiSoft) + ", inset 0 -1px 0 rgb(var(--border-subtle))",
      },
      sidebar: {
        background: "rgb(var(--bg-sidebar) / " + (d ? 0.55 : 0.66) + ")",
        backdropFilter: blur,
        WebkitBackdropFilter: blur,
        boxShadow: "inset -1px 0 0 rgb(var(--border-subtle)), " + topEdge(hiSoft),
      },
      listPane: { background: "rgb(var(--bg-primary))" },
      readingPane: { background: "rgb(var(--bg-primary))" },
      row: { background: "transparent" },
      rowSelected: {
        background: "rgb(var(--bg-surface))",
        boxShadow: topEdge(hiSoft) + ", " + botEdge(lo * 0.5),
      },
      floatWin: {
        background: "rgb(var(--bg-primary) / " + (d ? 0.72 : 0.78) + ")",
        backdropFilter: "blur(36px) saturate(180%)",
        WebkitBackdropFilter: "blur(36px) saturate(180%)",
        border: "1px solid rgb(255 255 255 / " + glassRing + ")",
        boxShadow:
          "0 28px 70px -18px rgb(0 0 0 / " + (d ? 0.75 : 0.28) + "), 0 2px 10px rgb(0 0 0 / " + (d ? 0.45 : 0.14) + "), " + topEdge(hi),
      },
      floatHeader: { background: "rgb(var(--bg-elevated) / 0.55)", boxShadow: "inset 0 -1px 0 rgb(var(--border-subtle))" },
      palette: {
        background: "rgb(var(--bg-elevated) / " + (d ? 0.68 : 0.76) + ")",
        backdropFilter: "blur(44px) saturate(190%)",
        WebkitBackdropFilter: "blur(44px) saturate(190%)",
        border: "1px solid rgb(255 255 255 / " + glassRing + ")",
        boxShadow: "0 32px 80px -20px rgb(0 0 0 / " + (d ? 0.8 : 0.3) + "), " + topEdge(hi),
      },
    };
  }

  if (variant === "B") {
    // Specular depth, zero blur: dimension from edge lighting alone.
    const bevel = topEdge(hi) + ", " + botEdge(lo);
    return {
      id: "B",
      title: "Specular depth · no blur",
      thesis:
        "Dimension from edge lighting alone: a 1px lit top edge, a 1px shaded bottom edge, layered elevation shadows. Every surface stays opaque — zero backdrop-filter.",
      verdict:
        "0 blur layers. Costs no compositing layer, survives the virtualizer, and is the only one that holds up in light mode. Rows read as raised keys rather than glass.",
      rowMode: "raised",
      titlebar: {
        background: "linear-gradient(180deg, rgb(var(--bg-elevated)), rgb(var(--bg-primary)))",
        boxShadow: bevel,
      },
      sidebar: {
        background: "linear-gradient(180deg, rgb(var(--bg-sidebar)), rgb(var(--bg-primary)))",
        boxShadow: "inset -1px 0 0 rgb(var(--border-subtle)), " + topEdge(hiSoft),
      },
      listPane: { background: "rgb(var(--bg-sidebar))" },
      readingPane: { background: "rgb(var(--bg-sidebar))" },
      row: {
        background: "linear-gradient(180deg, rgb(var(--bg-surface)), rgb(var(--bg-primary)))",
        borderRadius: "var(--glass-radius)",
        boxShadow:
          bevel + ", 0 1px 2px rgb(0 0 0 / " + (d ? 0.4 : 0.07) + "), 0 8px 18px -12px rgb(0 0 0 / " + (d ? 0.65 : 0.14) + ")",
      },
      rowSelected: {
        background: "linear-gradient(180deg, rgb(var(--bg-elevated)), rgb(var(--bg-surface)))",
        borderRadius: "var(--glass-radius)",
        boxShadow:
          topEdge(d ? 0.16 : 0.95) + ", " + botEdge(lo) + ", 0 0 0 1px rgb(var(--accent) / 0.55), 0 10px 24px -12px rgb(0 0 0 / " + (d ? 0.7 : 0.18) + ")",
      },
      floatWin: {
        background: "rgb(var(--bg-primary))",
        boxShadow:
          bevel + ", " + ring(d ? 0.06 : 0) + ", 0 0 0 1px rgb(var(--border-default)), 0 30px 60px -20px rgb(0 0 0 / " + (d ? 0.8 : 0.3) + ")",
      },
      floatHeader: {
        background: "linear-gradient(180deg, rgb(var(--bg-elevated)), rgb(var(--bg-surface)))",
        boxShadow: topEdge(hi) + ", inset 0 -1px 0 rgb(var(--border-default))",
      },
      palette: {
        background: "rgb(var(--bg-elevated))",
        boxShadow:
          bevel + ", 0 0 0 1px rgb(var(--border-default)), 0 34px 70px -22px rgb(0 0 0 / " + (d ? 0.85 : 0.32) + ")",
      },
    };
  }

  // C — full glass, including every row. The literal version of the ask.
  const rowBlur = "blur(16px) saturate(165%)";
  return {
    id: "C",
    title: "Full glass · every row",
    thesis:
      "Every surface translucent, including each message row — floating glass cards over the wallpaper, with a cursor-tracked 3D tilt and a specular sheen on hover.",
    verdict:
      "One blur layer per visible row plus chrome. Watch the FPS while scrolling, then switch density to ultra-compact and watch the cards collide.",
    rowMode: "floating",
    titlebar: {
      background: "rgb(var(--bg-primary) / " + (d ? 0.4 : 0.55) + ")",
      backdropFilter: "blur(30px) saturate(180%)",
      WebkitBackdropFilter: "blur(30px) saturate(180%)",
      boxShadow: topEdge(hi),
    },
    sidebar: {
      background: "rgb(var(--bg-sidebar) / " + (d ? 0.38 : 0.5) + ")",
      backdropFilter: "blur(40px) saturate(190%)",
      WebkitBackdropFilter: "blur(40px) saturate(190%)",
      boxShadow: "inset -1px 0 0 rgb(255 255 255 / " + glassRing * 0.6 + "), " + topEdge(hi),
    },
    listPane: { background: "rgb(var(--bg-primary) / " + (d ? 0.18 : 0.3) + ")" },
    readingPane: {
      background: "rgb(var(--bg-primary) / " + (d ? 0.34 : 0.46) + ")",
      backdropFilter: "blur(28px) saturate(170%)",
      WebkitBackdropFilter: "blur(28px) saturate(170%)",
      boxShadow: topEdge(hi) + ", inset 1px 0 0 rgb(255 255 255 / " + glassRing * 0.5 + ")",
    },
    row: {
      background: "rgb(var(--bg-surface) / " + (d ? 0.38 : 0.5) + ")",
      backdropFilter: rowBlur,
      WebkitBackdropFilter: rowBlur,
      borderRadius: "var(--glass-radius)",
      border: "1px solid rgb(255 255 255 / " + glassRing + ")",
      boxShadow: topEdge(hi) + ", 0 10px 26px -14px rgb(0 0 0 / " + (d ? 0.7 : 0.2) + ")",
    },
    rowSelected: {
      background: "rgb(var(--bg-elevated) / " + (d ? 0.55 : 0.68) + ")",
      backdropFilter: rowBlur,
      WebkitBackdropFilter: rowBlur,
      borderRadius: "var(--glass-radius)",
      border: "1px solid rgb(var(--accent) / 0.6)",
      boxShadow: topEdge(d ? 0.18 : 0.95) + ", 0 14px 34px -14px rgb(0 0 0 / " + (d ? 0.75 : 0.24) + ")",
    },
    floatWin: {
      background: "rgb(var(--bg-primary) / " + (d ? 0.55 : 0.66) + ")",
      backdropFilter: "blur(40px) saturate(190%)",
      WebkitBackdropFilter: "blur(40px) saturate(190%)",
      border: "1px solid rgb(255 255 255 / " + glassRing + ")",
      boxShadow:
        "0 30px 74px -18px rgb(0 0 0 / " + (d ? 0.8 : 0.3) + "), " + topEdge(hi),
    },
    floatHeader: { background: "rgb(255 255 255 / " + (d ? 0.05 : 0.28) + ")", boxShadow: botEdge(0.12) },
    palette: {
      background: "rgb(var(--bg-elevated) / " + (d ? 0.55 : 0.68) + ")",
      backdropFilter: "blur(48px) saturate(200%)",
      WebkitBackdropFilter: "blur(48px) saturate(200%)",
      border: "1px solid rgb(255 255 255 / " + glassRing + ")",
      boxShadow: "0 36px 84px -20px rgb(0 0 0 / " + (d ? 0.85 : 0.32) + "), " + topEdge(hi),
    },
  };
}

/* ── Shared pieces ──────────────────────────────────────────── */

function Wallpaper({ theme }: { theme: Theme }) {
  const dark =
    "radial-gradient(1100px 760px at 12% 8%, rgb(126 92 54 / .55), transparent 62%)," +
    "radial-gradient(900px 700px at 88% 18%, rgb(48 74 112 / .5), transparent 64%)," +
    "radial-gradient(1000px 900px at 55% 105%, rgb(150 104 62 / .42), transparent 66%)," +
    "linear-gradient(158deg, #1d1913, #0c0a08 55%, #15110c)";
  const light =
    "radial-gradient(1100px 760px at 12% 8%, rgb(255 214 160 / .85), transparent 62%)," +
    "radial-gradient(900px 700px at 88% 18%, rgb(178 206 240 / .8), transparent 64%)," +
    "radial-gradient(1000px 900px at 55% 105%, rgb(255 196 148 / .7), transparent 66%)," +
    "linear-gradient(158deg, #f6e9d8, #e6dccd 55%, #f2e6d6)";
  return <div className="absolute inset-0" style={{ background: theme === "dark" ? dark : light }} />;
}

function Avatar({ initials, mode }: { initials: string; mode: Kit["rowMode"] }) {
  return (
    <div
      className="msg-avatar flex shrink-0 items-center justify-center rounded-full text-[11px] font-semibold text-content-secondary"
      style={{
        width: "var(--avatar-size)",
        height: "var(--avatar-size)",
        background: mode === "floating" ? "rgb(255 255 255 / 0.10)" : "rgb(var(--bg-elevated))",
        boxShadow: mode === "flush" ? undefined : "inset 0 1px 0 rgb(255 255 255 / 0.10)",
      }}
    >
      {initials}
    </div>
  );
}

function Row({
  msg,
  kit,
  selected,
  onSelect,
}: {
  msg: MockMessage;
  kit: Kit;
  selected: boolean;
  onSelect: () => void;
}) {
  const [tilt, setTilt] = useState({ rx: 0, ry: 0, mx: 50, my: 50, on: false });
  const tiltable = kit.rowMode === "floating";

  const onMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!tiltable) return;
    const r = e.currentTarget.getBoundingClientRect();
    const px = (e.clientX - r.left) / r.width;
    const py = (e.clientY - r.top) / r.height;
    setTilt({ rx: (0.5 - py) * 5, ry: (px - 0.5) * 7, mx: px * 100, my: py * 100, on: true });
  };

  const base = selected ? kit.rowSelected : kit.row;
  const style: CSSProperties = {
    ...base,
    padding: "var(--row-padding-y) var(--row-padding-x)",
    fontSize: "var(--font-size-list)",
    transform: tilt.on
      ? "perspective(900px) rotateX(" + tilt.rx + "deg) rotateY(" + tilt.ry + "deg) translateZ(10px)"
      : undefined,
    transition: "transform 140ms ease-out, background 120ms ease",
    transformStyle: tiltable ? "preserve-3d" : undefined,
  };

  return (
    <div
      data-glass={base.backdropFilter ? "1" : undefined}
      onClick={onSelect}
      onMouseMove={onMove}
      onMouseLeave={() => setTilt((p) => ({ ...p, rx: 0, ry: 0, on: false }))}
      className="message-list-item relative flex shrink-0 cursor-pointer select-none items-start gap-2.5 overflow-hidden"
      style={style}
    >
      {tiltable && tilt.on && (
        <div
          className="pointer-events-none absolute inset-0"
          style={{
            background:
              "radial-gradient(260px circle at " + tilt.mx + "% " + tilt.my + "%, rgb(255 255 255 / 0.14), transparent 65%)",
          }}
        />
      )}
      <div className="flex shrink-0 items-start pt-1">
        {msg.unread ? (
          <div className="h-2 w-2 rounded-full bg-accent" />
        ) : (
          <div className="h-2 w-2" />
        )}
      </div>
      <Avatar initials={msg.initials} mode={kit.rowMode} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-2">
          <span className={"truncate " + (msg.unread ? "font-semibold text-content" : "text-content-secondary")}>
            {msg.from}
          </span>
          <div className="flex shrink-0 items-center gap-1.5">
            {msg.flagged && <Star className="h-3 w-3 fill-yellow-500 text-yellow-500" />}
            {msg.attachment && <Paperclip className="h-3 w-3 text-content-muted" />}
            <span className="text-[11px] text-content-muted">{msg.time}</span>
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          {msg.tag && (
            <span
              className="shrink-0 rounded px-1.5 py-0.5 text-[9px] font-medium leading-none text-content-secondary"
              style={{
                background: kit.rowMode === "floating" ? "rgb(255 255 255 / 0.12)" : "rgb(var(--bg-elevated))",
              }}
            >
              {msg.tag}
            </span>
          )}
          <span className={"truncate " + (msg.unread ? "text-content" : "text-content-secondary")}>
            {msg.subject}
          </span>
        </div>
        <div className="msg-snippet truncate text-content-muted" style={{ fontSize: "var(--font-size-snippet)" }}>
          {msg.snippet}
        </div>
      </div>
    </div>
  );
}

function ReadingPane({ kit }: { kit: Kit }) {
  const msg = MESSAGES[0];
  return (
    <div
      data-glass={kit.readingPane.backdropFilter ? "1" : undefined}
      className="flex min-w-0 flex-1 flex-col overflow-hidden"
      style={kit.readingPane}
    >
      <div className="shrink-0 px-5 pb-3 pt-4" style={{ boxShadow: "inset 0 -1px 0 rgb(var(--border-subtle))" }}>
        <div className="mb-1.5 text-[15px] font-semibold text-content">{msg.subject}</div>
        <div className="flex items-center gap-2.5">
          <Avatar initials={msg.initials} mode={kit.rowMode} />
          <div className="min-w-0">
            <div className="text-[13px] text-content">{msg.from}</div>
            <div className="text-[11px] text-content-muted">dana@northwind.example — to me</div>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-auto p-5">
        {/* The point of this block: the body is a sandboxed srcdoc iframe, so it
            can never be translucent no matter what the frame around it does. */}
        <div
          className="relative p-5 text-[13px] leading-relaxed"
          style={{
            borderRadius: "var(--glass-radius)",
            background: "#ffffff",
            color: "#1f2937",
            boxShadow: "0 1px 3px rgb(0 0 0 / 0.3), 0 10px 30px -14px rgb(0 0 0 / 0.5)",
          }}
        >
          <span
            className="absolute right-2 top-2 rounded px-1.5 py-0.5 text-[9px] font-medium"
            style={{ background: "#eef2f7", color: "#64748b" }}
          >
            sandboxed iframe — always opaque
          </span>
          <p className="mb-3">Chris —</p>
          <p className="mb-3">
            Sam and I talked it through this morning. If we can get the hosting piece locked by the
            12th, we can start recording the first four videos the week after.
          </p>
          <p className="mb-3">
            One open question on the number of presenters — the plan assumes two, but if David is going
            to record his own sections we probably want three.
          </p>
          <p className="mb-3">Let me know what that does to the number and I'll take it to the team.</p>
          <p className="text-[#64748b]">— Dana</p>
        </div>
      </div>
    </div>
  );
}

function FloatWindow({ kit, onClose }: { kit: Kit; onClose: () => void }) {
  return (
    <div
      data-glass={kit.floatWin.backdropFilter ? "1" : undefined}
      className="absolute z-30 flex flex-col overflow-hidden"
      style={{ ...kit.floatWin, borderRadius: "var(--glass-radius)", right: 28, bottom: 26, width: 380, height: 260 }}
    >
      <div className="flex h-9 shrink-0 cursor-grab items-center justify-between px-3" style={kit.floatHeader}>
        <span className="truncate text-xs font-medium text-content-secondary">
          Re: Portal rebuild — phase two scope
        </span>
        <div className="flex items-center gap-1">
          <Minus className="h-3 w-3 text-content-muted" />
          <X className="h-3 w-3 cursor-pointer text-content-muted" onClick={onClose} />
        </div>
      </div>
      <div className="flex-1 p-3.5 text-[13px] leading-relaxed text-content-secondary">
        <p className="mb-2">Morgan —</p>
        <p className="mb-2">
          Yes, let's quote those separately. Bundling reporting into the directory work makes it hard to
          say what either one is worth.
        </p>
        <p className="text-content-muted">— Chris</p>
      </div>
      <div className="flex shrink-0 items-center gap-2 px-3.5 pb-3.5">
        <button className="rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-white">Send</button>
        <button
          className="rounded-md px-3 py-1.5 text-xs text-content-secondary"
          style={{ boxShadow: "inset 0 0 0 1px rgb(var(--border-default))" }}
        >
          Save draft
        </button>
      </div>
    </div>
  );
}

function Palette({ kit, onClose }: { kit: Kit; onClose: () => void }) {
  return (
    <div className="absolute inset-0 z-40 flex items-start justify-center pt-24" onClick={onClose}>
      <div className="absolute inset-0" style={{ background: "rgb(0 0 0 / 0.28)" }} />
      <div
        data-glass={kit.palette.backdropFilter ? "1" : undefined}
        className="relative w-[460px] overflow-hidden"
        style={{ ...kit.palette, borderRadius: "var(--glass-radius)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 px-4 py-3" style={{ boxShadow: "inset 0 -1px 0 rgb(var(--border-subtle))" }}>
          <Search className="h-4 w-4 text-content-muted" />
          <span className="text-[13px] text-content-muted">Type a command or search…</span>
        </div>
        <div className="p-1.5">
          {PALETTE_ITEMS.map((item, i) => (
            <div
              key={item}
              className="flex items-center gap-2.5 px-3 py-2 text-[13px]"
              style={{
                borderRadius: "var(--glass-radius)",
                ...(i === 0
                  ? { background: "rgb(var(--accent) / 0.18)", color: "rgb(var(--text-primary))" }
                  : { color: "rgb(var(--text-secondary))" }),
              }}
            >
              <Sparkles className="h-3.5 w-3.5 opacity-60" />
              {item}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/* ── The shell every variant renders ────────────────────────── */

function Shell({
  kit,
  selected,
  onSelect,
  showWindow,
  onCloseWindow,
  showPalette,
  onClosePalette,
}: {
  kit: Kit;
  selected: number;
  onSelect: (i: number) => void;
  showWindow: boolean;
  onCloseWindow: () => void;
  showPalette: boolean;
  onClosePalette: () => void;
}) {
  const floating = kit.rowMode === "floating";
  const raised = kit.rowMode === "raised";
  const gap = floating ? 8 : raised ? 6 : 0;

  return (
    <div className="relative flex h-full flex-col overflow-hidden">
      {/* title bar */}
      <div
        data-glass={kit.titlebar.backdropFilter ? "1" : undefined}
        className="relative z-20 flex h-9 shrink-0 items-center justify-between pl-20 pr-3"
        style={kit.titlebar}
      >
        <span className="text-xs font-medium text-content-muted">Inbox — 12 unread</span>
        <div className="flex items-center gap-1.5 text-xs text-content-secondary">
          <PenSquare className="h-3.5 w-3.5" />
          Compose
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        {/* sidebar */}
        <div
          data-glass={kit.sidebar.backdropFilter ? "1" : undefined}
          className="w-[188px] shrink-0 overflow-hidden p-2.5"
          style={kit.sidebar}
        >
          <div className="mb-3 px-2 text-[10px] font-semibold uppercase tracking-wider text-content-faint">
            Accounts
          </div>
          {FOLDERS.map((f, i) => {
            const Icon = f.icon;
            const active = i === 0;
            return (
              <div
                key={f.label}
                className="sidebar-item mb-0.5 flex items-center gap-2.5 px-2.5 text-[13px]"
                style={{
                  borderRadius: "var(--glass-radius)",
                  ...(active
                    ? {
                        background:
                          kit.id === "B" ? "rgb(var(--bg-elevated))" : "rgb(255 255 255 / " + (kit.id === "C" ? 0.14 : 0.07) + ")",
                        boxShadow: "inset 0 1px 0 rgb(255 255 255 / 0.10)",
                        color: "rgb(var(--text-primary))",
                      }
                    : { color: "rgb(var(--text-secondary))" }),
                }}
              >
                <Icon className="h-3.5 w-3.5 opacity-70" />
                <span className="flex-1 truncate">{f.label}</span>
                {f.count > 0 && <span className="text-[11px] text-content-muted">{f.count}</span>}
              </div>
            );
          })}
        </div>

        {/* message list */}
        <div
          className="flex w-[352px] shrink-0 flex-col overflow-hidden"
          style={{ ...kit.listPane, boxShadow: "inset -1px 0 0 rgb(var(--border-subtle))" }}
        >
          <div
            className="flex min-h-0 flex-1 flex-col overflow-y-auto"
            style={{ gap, padding: gap ? gap : 0 }}
            data-testid="scroll-list"
          >
            {MESSAGES.map((m, i) => (
              <Row key={m.id} msg={m} kit={kit} selected={i === selected} onSelect={() => onSelect(i)} />
            ))}
          </div>
        </div>

        <ReadingPane kit={kit} />
      </div>

      {showWindow && <FloatWindow kit={kit} onClose={onCloseWindow} />}
      {showPalette && <Palette kit={kit} onClose={onClosePalette} />}
    </div>
  );
}

/* ── Variants ───────────────────────────────────────────────── */

type VariantProps = Omit<Parameters<typeof Shell>[0], "kit"> & { theme: Theme };

function MockA(p: VariantProps) {
  return <Shell {...p} kit={buildKit("A", p.theme)} />;
}

function MockB(p: VariantProps) {
  return <Shell {...p} kit={buildKit("B", p.theme)} />;
}

function MockC(p: VariantProps) {
  return <Shell {...p} kit={buildKit("C", p.theme)} />;
}

/* ── Harness ────────────────────────────────────────────────── */

const VARIANTS: Variant[] = ["A", "B", "C"];
const DENSITIES: Density[] = ["comfortable", "compact", "ultra-compact"];

export default function GlassHarness() {
  const [variant, setVariant] = useState<Variant>("C");
  const [radius, setRadius] = useState(8);
  const [theme, setTheme] = useState<Theme>("dark");
  const [density, setDensity] = useState<Density>("comfortable");
  const [selected, setSelected] = useState(2);
  const [showWindow, setShowWindow] = useState(true);
  const [showPalette, setShowPalette] = useState(false);
  const [layers, setLayers] = useState(0);
  const [fps, setFps] = useState(60);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    document.documentElement.dataset.density = density;
  }, [density]);

  // Rolling FPS — the point of variant C is that this number moves while scrolling.
  useEffect(() => {
    let raf = 0;
    let frames = 0;
    let last = performance.now();
    const loop = (t: number) => {
      frames++;
      if (t - last >= 500) {
        setFps(Math.round((frames * 1000) / (t - last)));
        frames = 0;
        last = t;
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, []);

  useLayoutEffect(() => {
    setLayers(document.querySelectorAll("[data-glass='1']").length);
  }, [variant, theme, density, showWindow, showPalette]);

  const kit = buildKit(variant, theme);
  const shellProps = {
    theme,
    selected,
    onSelect: setSelected,
    showWindow,
    onCloseWindow: () => setShowWindow(false),
    showPalette,
    onClosePalette: () => setShowPalette(false),
  };

  return (
    <div
      className="min-h-screen bg-[#0b0a09] p-6 font-sans"
      style={
        {
          "--glass-radius": radius + "px",
        } as CSSProperties
      }
    >
      <div className="mx-auto max-w-[1360px]">
        {/* controls */}
        <div className="mb-4 flex flex-wrap items-center gap-2">
          {VARIANTS.map((v) => (
            <button
              key={v}
              data-testid={"variant-" + v}
              onClick={() => setVariant(v)}
              className={
                "rounded-lg px-3 py-2 text-sm font-medium transition-colors " +
                (variant === v ? "bg-accent text-white" : "bg-white/8 text-white/60 hover:bg-white/12")
              }
            >
              {v} — {buildKit(v, theme).title}
            </button>
          ))}

          <div className="ml-auto flex items-center gap-2">
            <button
              data-testid="toggle-theme"
              onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
              className="rounded-lg bg-white/8 px-3 py-2 text-xs text-white/70 hover:bg-white/12"
            >
              {theme}
            </button>
            <select
              data-testid="density"
              value={density}
              onChange={(e) => setDensity(e.target.value as Density)}
              className="rounded-lg bg-white/8 px-2.5 py-2 text-xs text-white/70 outline-none"
            >
              {DENSITIES.map((d) => (
                <option key={d} value={d} className="bg-[#1c1a17]">
                  {d}
                </option>
              ))}
            </select>
            <label className="flex items-center gap-2 rounded-lg bg-white/8 px-2.5 py-2 text-xs text-white/70">
              <span className="whitespace-nowrap">radius</span>
              <input
                data-testid="radius"
                type="range"
                min={0}
                max={14}
                step={1}
                value={radius}
                onChange={(e) => setRadius(Number(e.target.value))}
                className="h-1 w-24 accent-[rgb(10_132_255)]"
              />
              <span className="w-8 tabular-nums text-white/45">{radius}px</span>
            </label>
            <button
              data-testid="toggle-palette"
              onClick={() => setShowPalette((s) => !s)}
              className="rounded-lg bg-white/8 px-3 py-2 text-xs text-white/70 hover:bg-white/12"
            >
              ⌘K
            </button>
            <button
              data-testid="toggle-window"
              onClick={() => setShowWindow((s) => !s)}
              className="rounded-lg bg-white/8 px-3 py-2 text-xs text-white/70 hover:bg-white/12"
            >
              window
            </button>
          </div>
        </div>

        {/* meters */}
        <div className="mb-3 flex flex-wrap items-center gap-4 text-[11px]">
          <span className="text-white/45">
            blur layers live:{" "}
            <span
              data-testid="layer-count"
              className={layers === 0 ? "font-semibold text-emerald-400" : layers > 10 ? "font-semibold text-red-400" : "font-semibold text-amber-300"}
            >
              {layers}
            </span>
          </span>
          <span className="text-white/45">
            fps: <span className={fps < 50 ? "font-semibold text-red-400" : "font-semibold text-emerald-400"}>{fps}</span>
          </span>
          <span className="text-white/30">scroll the message list to load the compositor</span>
        </div>

        {/* stage — wallpaper behind a simulated transparent window */}
        <div className="relative overflow-hidden rounded-2xl" style={{ height: 660, boxShadow: "0 40px 90px -30px rgb(0 0 0 / 0.9)" }}>
          <Wallpaper theme={theme} />
          <div className="absolute inset-0 p-8">
            <div
              className="h-full overflow-hidden"
              style={{
                borderRadius: 10,
                boxShadow: "0 0 0 1px rgb(255 255 255 / 0.10), 0 30px 70px -20px rgb(0 0 0 / 0.75)",
              }}
            >
              {variant === "A" && <MockA {...shellProps} />}
              {variant === "B" && <MockB {...shellProps} />}
              {variant === "C" && <MockC {...shellProps} />}
            </div>
          </div>
        </div>

        {/* per-variant notes */}
        <div className="mt-4 rounded-xl bg-white/5 p-4">
          <div className="mb-1 text-sm font-semibold text-white/90">
            {kit.id} — {kit.title}
          </div>
          <p className="mb-2 text-[13px] leading-relaxed text-white/60">{kit.thesis}</p>
          <p className="text-[13px] leading-relaxed text-white/45">{kit.verdict}</p>
        </div>

        <p className="mt-4 text-center text-[11px] text-white/25">
          Mock exploration page — data is static.
        </p>
      </div>
    </div>
  );
}
