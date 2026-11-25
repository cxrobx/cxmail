#!/usr/bin/env python3
"""Read-only latency + IMAP-login eval for the phone's message-open paths.

Drives the live remote API the way the iOS app does and reports, per scenario,
how long it took and how many NEW TCP connections to an IMAP port (993) any
process opened meanwhile -- each one is a login against the account's
simultaneous-connection cap.

Scenarios, each on messages whose body the desktop has NOT cached yet:
  preview  long-press: /api/message?policy=0, then /api/body
  open     tap:        /api/message, then /api/body?images=auto
  reopen   tap again:  the same two calls
  burst    10 long-presses on 10 different uncached messages, back to back

It never mutates mail: GETs only. (Since the body-persist change, a GET that
fetches a body stores it in the desktop cache -- the same write opening the
message on the desktop makes.)

    CXMAIL_REMOTE_URL=https://<host>.ts.net:<port> \
      python3 remote/evals/preview_latency.py --label after --skip 0 --out result.json

`--skip N` skips the first N uncached candidates, so two runs can use
disjoint messages. Must run on the mini (it reads the DB to find uncached
rows, and watches local sockets).
"""
import argparse, json, os, sqlite3, statistics, subprocess, threading, time, urllib.parse, urllib.request

DB = os.path.expanduser("~/Library/Application Support/com.cxmail.app/cxmail.db")
MCP_LOG = os.path.expanduser("~/Library/Logs/mcp-cxmail.log")


class ImapConnWatcher(threading.Thread):
    """Polls lsof; a (pid, local port) pair to :993 not seen before = a new login."""

    def __init__(self):
        super().__init__(daemon=True)
        self.seen, self.events, self.stop, self.lock = set(), [], False, threading.Lock()
        self.seen |= self.snapshot()

    @staticmethod
    def snapshot():
        out = subprocess.run(["lsof", "-nP", "-iTCP:993", "-F", "pcn"], capture_output=True, text=True).stdout
        pairs, pid, cmd = set(), None, None
        for line in out.splitlines():
            if line.startswith("p"): pid = line[1:]
            elif line.startswith("c"): cmd = line[1:]
            elif line.startswith("n") and "->" in line:
                pairs.add((pid, cmd, line[1:].split("->")[0]))
        return pairs

    def run(self):
        while not self.stop:
            now = self.snapshot()
            with self.lock:
                for p in now - self.seen:
                    self.events.append((time.monotonic(), p[1]))
                self.seen |= now
            time.sleep(0.05)

    def count_since(self, t0):
        time.sleep(0.4)  # let the poller catch a connection opened at the very end
        with self.lock:
            hits = [e for e in self.events if e[0] >= t0]
        by = {}
        for _, cmd in hits:
            by[cmd] = by.get(cmd, 0) + 1
        return len(hits), by


def mcp_logins():
    try:
        with open(MCP_LOG, errors="replace") as f:
            return sum(1 for line in f if "IMAP greeting" in line)
    except OSError:
        return 0


def get(base, path, query):
    url = base + path + "?" + urllib.parse.urlencode(query)
    t = time.monotonic()
    with urllib.request.urlopen(url, timeout=240) as r:
        body = r.read()
        status = r.status
    return time.monotonic() - t, status, body


def uncached(base, need, skip):
    conn = sqlite3.connect("file:{}?mode=ro".format(DB), uri=True)
    out, before = [], None
    for _ in range(12):
        q = {"view": "primary"}
        if before: q["before"] = before
        _, _, body = get(base, "/api/messages", q)
        rows = json.loads(body)
        if not rows: break
        for m in rows:
            hit = conn.execute("SELECT 1 FROM message_bodies WHERE account_id=? AND folder_name=? AND uid=?",
                               (m["account"], m["folder"], m["uid"])).fetchone()
            if not hit and not m.get("draft"):
                out.append({"account": m["account"], "folder": m["folder"], "uid": m["uid"]})
        before = rows[-1]["date"]
        if len(out) >= skip + need: break
    return out[skip:skip + need]


def preview(base, ref):
    t1, s1, b1 = get(base, "/api/message", dict(ref, policy="0"))
    steps = {"message": t1}
    if s1 == 200 and json.loads(b1).get("has_html"):
        steps["body"] = get(base, "/api/body", ref)[0]
    return steps


def open_(base, ref):
    t1, s1, b1 = get(base, "/api/message", ref)
    steps = {"message": t1}
    if s1 == 200 and json.loads(b1).get("has_html"):
        steps["body_auto"] = get(base, "/api/body", dict(ref, images="auto"))[0]
    return steps


def scenario(watch, fn, *args):
    t0, m0 = time.monotonic(), mcp_logins()
    steps = fn(*args)
    total = time.monotonic() - t0
    logins, by = watch.count_since(t0)
    return {"total": round(total, 3), "steps": {k: round(v, 3) for k, v in steps.items()},
            "imap_logins": logins, "by_process": by, "mcp_log_logins": mcp_logins() - m0}


def summarize(rows):
    if not rows: return {}
    tot = [r["total"] for r in rows]
    return {"n": len(rows), "median_s": round(statistics.median(tot), 3), "max_s": round(max(tot), 3),
            "logins_total": sum(r["imap_logins"] for r in rows),
            "logins_per_msg": round(sum(r["imap_logins"] for r in rows) / len(rows), 2)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", required=True)
    ap.add_argument("--messages", type=int, default=6)
    ap.add_argument("--burst", type=int, default=10)
    ap.add_argument("--skip", type=int, default=0)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    base = os.environ["CXMAIL_REMOTE_URL"].rstrip("/")

    refs = uncached(base, a.messages + a.burst, a.skip)
    if len(refs) < a.messages + a.burst:
        raise SystemExit("only {} uncached messages found".format(len(refs)))
    watch = ImapConnWatcher(); watch.start()
    per = {"preview": [], "open": [], "reopen": []}
    for ref in refs[:a.messages]:
        per["preview"].append(dict(scenario(watch, preview, base, ref), uid=ref["uid"]))
        per["open"].append(dict(scenario(watch, open_, base, ref), uid=ref["uid"]))
        per["reopen"].append(dict(scenario(watch, open_, base, ref), uid=ref["uid"]))
    burst_refs = refs[a.messages:]
    burst = scenario(watch, lambda: {"each": sum(sum(preview(base, r).values()) for r in burst_refs)})
    watch.stop = True

    report = {"label": a.label, "at": time.strftime("%Y-%m-%dT%H:%M:%S"),
              "summary": {k: summarize(v) for k, v in per.items()},
              "burst": {"messages": len(burst_refs), "total_s": burst["total"],
                        "imap_logins": burst["imap_logins"], "by_process": burst["by_process"]},
              "detail": per}
    with open(a.out, "w") as f:
        json.dump(report, f, indent=2)
    print(json.dumps({"label": a.label, "summary": report["summary"], "burst": report["burst"]}, indent=2))


if __name__ == "__main__":
    main()
