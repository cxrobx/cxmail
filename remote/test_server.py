#!/usr/bin/env python3
"""Tests for CXMail Remote. Run: /usr/bin/python3 remote/test_server.py

Uses a throwaway fixture database (fiction only: this repo is public) and a
stubbed MCP. One extra test checks every column the server reads still exists
in the real mailbox's schema, when that database is on this machine. It opens
the database read-only.
"""

import json
import os
import sqlite3
import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request

TMP = tempfile.mkdtemp()
FIXTURE = os.path.join(TMP, "cxmail.db")
# A stand-in for `cxmail-mcp remote ...`: logs its argv, answers like the real one.
FAKE_BIN = os.path.join(TMP, "fake-cxmail-mcp")
CALLS = os.path.join(TMP, "calls.log")
DIGEST = "a" * 64
with open(FAKE_BIN, "w") as f:
    f.write("""#!/bin/sh
echo "$@" >> %s
case "$2" in
  preview-draft) echo '{"account_id":"'$3'","folder":"'$4'","uid":'$5',"from":{"name":null,"email":"me@example.com"},"to":[{"name":"Ann","email":"ann@example.org"}],"cc":[],"bcc":[{"name":null,"email":"b@example.org"}],"subject":"Hi","sanitized_html":"<p>body</p>","plain_text":"body","attachments":[],"sendable":true,"blocked_reason":null,"digest":"%s"}' ;;
  send-draft) if [ "$6" = "%s" ]; then echo '{"message_id":"<x@cxmail.app>","draft_removed":true,"note":null}'; else echo '{"error":"The draft changed after you reviewed it."}'; exit 1; fi ;;
  silent) exit 1 ;;
  *) echo '{"error":"usage: cxmail-mcp remote ..."}'; exit 1 ;;
esac
""" % (CALLS, DIGEST, DIGEST))
os.chmod(FAKE_BIN, 0o755)
os.environ.update(
    {
        "CXMAIL_DB": FIXTURE,
        "CXMAIL_REMOTE_LOGINS": "owner@example.com",
        "CXMAIL_REMOTE_PORT": "0",
        "CXMAIL_MCP_URL": "http://127.0.0.1:9/never",
        "CXMAIL_SEND_BIN": FAKE_BIN,
    }
)
os.environ.pop("CXMAIL_REMOTE_DEV", None)
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import server  # noqa: E402

SCHEMA = """
CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT, display_name TEXT, color TEXT,
  is_active INTEGER DEFAULT 1, sort_order INTEGER DEFAULT 0, hidden_from_aggregates INTEGER DEFAULT 0);
CREATE TABLE folders (account_id TEXT, name TEXT, display_name TEXT, folder_type TEXT);
CREATE TABLE messages (id INTEGER PRIMARY KEY, account_id TEXT, folder_name TEXT, uid INTEGER,
  message_id TEXT, subject TEXT, from_name TEXT, from_email TEXT, to_list TEXT, cc_list TEXT,
  date TEXT, snippet TEXT, is_read INTEGER DEFAULT 0, is_flagged INTEGER DEFAULT 0,
  has_attachments INTEGER DEFAULT 0, category TEXT, thread_root_id TEXT);
CREATE TABLE message_bodies (account_id TEXT, folder_name TEXT, uid INTEGER, plain_text TEXT,
  sanitized_html TEXT, id INTEGER PRIMARY KEY);
CREATE TABLE attachments (account_id TEXT, folder_name TEXT, message_uid INTEGER, filename TEXT,
  is_inline INTEGER DEFAULT 0);
CREATE TABLE trusted_image_senders (sender_email TEXT PRIMARY KEY);
CREATE VIRTUAL TABLE messages_fts USING fts5(subject, sender, recipients, body, attachment_names);
"""


def build_fixture():
    c = sqlite3.connect(FIXTURE)
    c.executescript(SCHEMA)
    c.executemany(
        "INSERT INTO accounts (id, email, hidden_from_aggregates) VALUES (?, ?, ?)",
        [("a1", "me@example.com", 0), ("a2", "hidden@example.com", 1)],
    )
    c.executemany(
        "INSERT INTO folders VALUES (?, ?, ?, ?)",
        [
            ("a1", "INBOX", None, "inbox"),
            ("a1", "[Gmail]/All Mail", "All Mail", "archive"),
            ("a1", "[Gmail]/Trash", "Trash", "trash"),
            ("a1", "[Gmail]/Drafts", "Drafts", "drafts"),
            ("a2", "INBOX", None, "inbox"),
        ],
    )
    rows = [
        # id, acct, folder, uid, msgid, subject, from, date, read, cat, root
        (1, "a1", "INBOX", 10, "<m1@x>", "Quarterly budget", "ann@example.org", "2026-11-10T09:00:00+00:00", 0, "primary", "r1"),
        (2, "a1", "INBOX", 11, "<m2@x>", "Big sale today", "shop@example.org", "2026-11-11T09:00:00+00:00", 1, "promotions", None),
        (3, "a1", "[Gmail]/All Mail", 10, "<m1@x>", "Quarterly budget", "ann@example.org", "2026-11-10T09:00:00+00:00", 0, "primary", "r1"),
        (4, "a1", "[Gmail]/All Mail", 5, "<m0@x>", "Re: budget", "bob@example.org", "2026-11-09T09:00:00+00:00", 1, "primary", "r1"),
        (5, "a1", "[Gmail]/Trash", 6, "<m9@x>", "budget junk", "x@example.org", "2026-11-08T09:00:00+00:00", 1, "primary", "r1"),
        (7, "a1", "[Gmail]/Drafts", 3, "<d1@x>", "Re: budget", "me@example.com", "2026-11-12T10:00:00+00:00", 1, "primary", "r1"),
        (6, "a2", "INBOX", 1, "<h1@x>", "hidden budget", "h@example.org", "2026-11-12T09:00:00+00:00", 0, "primary", None),
    ]
    for (i, a, f, u, mid, s, fr, d, rd, cat, root) in rows:
        c.execute(
            "INSERT INTO messages (id, account_id, folder_name, uid, message_id, subject, from_email, "
            "to_list, cc_list, date, snippet, is_read, category, thread_root_id) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (i, a, f, u, mid, s, fr, json.dumps([{"email": "me@example.com", "name": None}]), "[]", d, "snip", rd, cat, root),
        )
        c.execute(
            "INSERT INTO messages_fts (rowid, subject, sender, recipients, body, attachment_names) VALUES (?,?,?,?,?,?)",
            (i, s, fr, "me@example.com", "", ""),
        )
    c.execute(
        "INSERT INTO message_bodies (account_id, folder_name, uid, plain_text, sanitized_html) VALUES "
        "('a1','INBOX',10,'hi','<p>hi <img src=\"https://t.example/p.gif\"></p>')"
    )
    c.execute("INSERT INTO attachments VALUES ('a1','INBOX',10,'plan.pdf',0), ('a1','INBOX',10,'logo.png',1)")
    c.commit()
    c.close()


class FtsParity(unittest.TestCase):
    """The cases from cxmail-db's build_match_expr tests, verbatim."""

    CASES = [
        ("budget", False, '"budget"'),
        ("budget report", False, '"budget" AND "report"'),
        ("budget (", False, '"budget"'),
        ("a AND b", False, '"a" AND "AND" AND "b"'),
        ("from:", False, '"from:"'),
        ("NEAR", False, '"NEAR"'),
        ("x^2*", False, '"x^2*"'),
        ('"quarterly budget" review', False, '"quarterly budget" AND "review"'),
        ('"open phrase', False, '"open phrase"'),
        ('ab"cd', False, '"ab""cd"'),
        ("budget -promo", False, '"budget" NOT "promo"'),
        ('budget -"special offer"', False, '"budget" NOT "special offer"'),
        ("-promo", False, None),
        ("e-mail", False, '"e-mail"'),
        ("budg", True, '"budg"*'),
        ("q3 budg", True, '"q3" AND "budg"*'),
        ("budg ", True, '"budg"*'),
        ("budg -promo", True, '"budg"* NOT "promo"'),
        ("café", False, '"café"'),
        ("日本語 メール", False, '"日本語" AND "メール"'),
        ("", False, None),
        ("   ", False, None),
        ("(((", False, None),
    ]

    def test_cases(self):
        for text, prefix, want in self.CASES:
            self.assertEqual(server.fts_query(text, prefix), want, text)


class Api(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        build_fixture()
        cls.calls = []

        def fake_call(tool, args):
            cls.calls.append((tool, args))
            return "ok"

        server.MCP.call = fake_call
        cls.srv = server.ThreadingHTTPServer(("127.0.0.1", 0), server.Handler)
        cls.base = "http://127.0.0.1:%d" % cls.srv.server_address[1]
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def setUp(self):
        server.PENDING.items.clear()

    def req(self, path, body=None, login="owner@example.com", headers=None):
        h = dict(headers or {})
        if login:
            h["Tailscale-User-Login"] = login
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            h.setdefault("Content-Type", "application/json")
            h.setdefault("X-CXMail-Remote", "1")
        r = urllib.request.Request(self.base + path, data, h)
        try:
            with urllib.request.urlopen(r) as resp:
                return resp.status, resp.headers, resp.read()
        except urllib.error.HTTPError as e:
            return e.code, e.headers, e.read()

    def j(self, path, **kw):
        code, _, raw = self.req(path, **kw)
        self.assertEqual(code, 200, raw)
        return json.loads(raw)

    # auth --------------------------------------------------------------
    def test_no_login_is_refused(self):
        for path in ("/", "/api/accounts", "/api/messages"):
            self.assertEqual(self.req(path, login=None)[0], 403, path)

    def test_wrong_login_is_refused(self):
        self.assertEqual(self.req("/api/accounts", login="someone@example.com")[0], 403)

    def test_login_is_case_insensitive(self):
        self.assertEqual(self.req("/api/accounts", login="Owner@Example.com")[0], 200)

    def test_post_needs_csrf_header_and_json(self):
        body = {"action": "archive", "account": "a1", "folder": "INBOX", "uid": 10}
        self.assertEqual(self.req("/api/action", body, headers={"X-CXMail-Remote": "0"})[0], 400)
        self.assertEqual(self.req("/api/action", body, headers={"Content-Type": "text/plain"})[0], 400)
        self.assertEqual(self.req("/api/action", body, login=None)[0], 403)

    # reads ---------------------------------------------------------------
    def test_primary_inbox_excludes_hidden_and_other_categories(self):
        rows = self.j("/api/messages?view=primary")
        self.assertEqual([(r["account"], r["uid"]) for r in rows], [("a1", 10)])

    def test_hidden_account_when_named(self):
        rows = self.j("/api/messages?view=all&account=a2")
        self.assertEqual([r["uid"] for r in rows], [1])

    def test_paging_before(self):
        rows = self.j("/api/messages?view=all&before=2026-11-11T00:00:00%2B00:00")
        self.assertEqual([r["uid"] for r in rows], [10])

    def test_search_skips_trash_and_hidden(self):
        rows = self.j("/api/messages?q=budg")
        got = sorted((r["folder"], r["uid"]) for r in rows)
        self.assertEqual(got, [("INBOX", 10), ("[Gmail]/All Mail", 5), ("[Gmail]/All Mail", 10)])

    def test_search_syntax_never_errors(self):
        for q in ['"', "(((", "a AND", "NEAR(", "-x", "*"]:
            self.assertEqual(self.req("/api/messages?q=" + urllib.request.quote(q))[0], 200, q)

    def test_message_thread_dedupes_and_skips_trash(self):
        m = self.j("/api/message?account=a1&folder=INBOX&uid=10")
        self.assertEqual(m["attachment_names"], ["plan.pdf"])
        self.assertEqual([(t["folder"], t["uid"]) for t in m["thread"]], [("[Gmail]/All Mail", 5)])
        self.assertFalse(m["images_trusted"])
        self.assertTrue(m["has_html"])

    def test_body_csp_blocks_scripts_and_remote_images(self):
        code, headers, raw = self.req("/api/body?account=a1&folder=INBOX&uid=10")
        self.assertEqual(code, 200)
        csp = headers["Content-Security-Policy"]
        self.assertIn("default-src 'none'", csp)
        self.assertNotIn("script-src", csp)
        self.assertIn("img-src data: cid:;", csp)
        self.assertIn("sandbox", csp)
        _, headers, _ = self.req("/api/body?account=a1&folder=INBOX&uid=10&images=1")
        self.assertIn("https:", headers["Content-Security-Policy"])

    def test_trusted_sender_images(self):
        c = sqlite3.connect(FIXTURE)
        c.execute("INSERT OR IGNORE INTO trusted_image_senders VALUES ('ann@example.org')")
        c.commit()
        c.close()
        self.assertTrue(self.j("/api/message?account=a1&folder=INBOX&uid=10")["images_trusted"])
        c = sqlite3.connect(FIXTURE)
        c.execute("DELETE FROM trusted_image_senders")
        c.commit()
        c.close()

    def test_database_is_read_only(self):
        with server.db() as c:
            with self.assertRaises(sqlite3.Error):
                c.execute("DELETE FROM messages")

    # writes go through the MCP ---------------------------------------------
    def test_actions_map_to_mcp_tools(self):
        self.calls.clear()
        base = {"account": "a1", "folder": "INBOX", "uid": 10}
        for action, extra in [("archive", {}), ("star", {}), ("unread", {}), ("move", {"to_folder": "Notes"})]:
            self.j("/api/action", body=dict(base, action=action, **extra))
        self.assertEqual(
            [(t, a.get("flag") or a.get("to_folder")) for t, a in self.calls],
            [("archive_email", None), ("flag_email", "starred"), ("flag_email", "unread"), ("move_email", "Notes")],
        )

    def test_changes_show_before_the_app_syncs(self):
        # The fixture DB never changes, just as the app's DB lags behind IMAP.
        base = {"account": "a1", "folder": "INBOX", "uid": 10}
        self.j("/api/action", body=dict(base, action="star"))
        self.assertTrue(self.j("/api/messages?view=primary")[0]["starred"])
        self.assertTrue(self.j("/api/message?account=a1&folder=INBOX&uid=10")["starred"])
        self.j("/api/action", body=dict(base, action="archive"))
        self.assertEqual(self.j("/api/messages?view=primary"), [])
        self.assertNotIn(("INBOX", 10), [(r["folder"], r["uid"]) for r in self.j("/api/messages?q=budget")])
        self.assertEqual(self.req("/api/message?account=a1&folder=INBOX&uid=10")[0], 404)

    def test_overlay_drops_once_the_db_agrees(self):
        server.PENDING.note(("a1", "INBOX", 10), read=False)  # fixture row is unread
        self.j("/api/messages?view=primary")
        self.assertEqual(server.PENDING.items, {})

    def test_failed_action_leaves_no_overlay(self):
        def boom(tool, args):
            raise server.McpError("down")
        saved, server.MCP.call = server.MCP.call, boom
        try:
            code, _, _ = self.req("/api/action", {"action": "archive", "account": "a1", "folder": "INBOX", "uid": 10})
        finally:
            server.MCP.call = saved
        self.assertEqual(code, 502)
        self.assertEqual(len(self.j("/api/messages?view=primary")), 1)

    # sending -----------------------------------------------------------------
    def test_drafts_tab_lists_drafts_only(self):
        rows = self.j("/api/drafts")
        self.assertEqual([(r["folder"], r["uid"], r["draft"]) for r in rows], [("[Gmail]/Drafts", 3, True)])

    def test_inbox_search_still_skips_drafts(self):
        self.assertNotIn("[Gmail]/Drafts", [r["folder"] for r in self.j("/api/messages?q=budget")])

    def test_review_then_send(self):
        p = self.j("/api/draft-preview?account=a1&folder=%5BGmail%5D%2FDrafts&uid=3")
        self.assertEqual([a["email"] for a in p["bcc"]], ["b@example.org"])
        self.assertNotIn("sanitized_html", p)  # the body comes through the CSP'd endpoint
        code, headers, raw = self.req("/api/draft-body?account=a1&folder=%5BGmail%5D%2FDrafts&uid=3&digest=" + p["digest"])
        self.assertEqual(code, 200)
        self.assertIn(b"<p>body</p>", raw)
        self.assertIn("default-src 'none'", headers["Content-Security-Policy"])
        out = self.j("/api/send", body={"account": "a1", "folder": "[Gmail]/Drafts", "uid": 3, "digest": p["digest"]})
        self.assertTrue(out["ok"] and out["draft_removed"])
        self.assertEqual(open(CALLS).read().splitlines()[-1], "remote send-draft a1 [Gmail]/Drafts 3 " + DIGEST)
        self.assertEqual(self.j("/api/drafts"), [])  # gone before the app syncs

    def test_send_refuses_changed_draft_and_bad_input(self):
        code, _, raw = self.req("/api/send", {"account": "a1", "folder": "[Gmail]/Drafts", "uid": 3, "digest": "b" * 64})
        self.assertEqual(code, 502)
        self.assertIn(b"changed", raw)
        self.assertEqual(len(self.j("/api/drafts")), 1)  # a refused send hides nothing
        self.assertEqual(self.req("/api/send", {"account": "a1", "folder": "[Gmail]/Drafts", "uid": 3})[0], 400)
        self.assertEqual(self.req("/api/send", {"account": "a1", "folder": "[Gmail]/Drafts", "uid": 3, "digest": DIGEST}, login=None)[0], 403)
        self.assertEqual(self.req("/api/send", {"account": "a1", "folder": "[Gmail]/Drafts", "uid": 3, "digest": DIGEST},
                                  headers={"X-CXMail-Remote": "0"})[0], 400)

    def test_old_cxmail_build_says_ship_first(self):
        with self.assertRaises(server.SendError) as cm:
            server.run_send_bin("not-a-command")
        self.assertIn("Ship the update", str(cm.exception))
        with self.assertRaises(server.SendError) as cm:
            server.run_send_bin("silent")  # what a pre-`remote` build does
        self.assertIn("Ship the update", str(cm.exception))

    def test_body_needs_the_reviewed_digest(self):
        self.assertEqual(self.req("/api/draft-body?account=a1&folder=x&uid=3&digest=nope")[0], 404)

    def test_unknown_action_and_bad_move(self):
        self.assertEqual(self.req("/api/action", {"action": "delete", "account": "a1", "folder": "INBOX", "uid": 1})[0], 400)
        self.assertEqual(self.req("/api/action", {"action": "move", "account": "a1", "folder": "INBOX", "uid": 1})[0], 400)

    def test_draft_never_sends_and_threads_replies(self):
        self.calls.clear()
        self.j("/api/draft", body={"account": "a1", "to": ["ann@example.org", "not-an-address"], "subject": "Re: x",
                                   "body": "ok", "reply_folder": "INBOX", "reply_uid": 10})
        tool, args = self.calls[-1]
        self.assertEqual(tool, "compose_draft")
        self.assertNotIn("send_email", [t for t, _ in self.calls])
        self.assertEqual(args["to"], ["ann@example.org"])
        self.assertEqual((args["reply_to_folder"], args["reply_to_uid"]), ("INBOX", 10))
        self.assertFalse(args["is_html"])
        self.assertNotIn("send_email", server.ACTIONS)
        self.assertEqual(self.req("/api/draft", {"account": "a1", "to": []})[0], 400)


REAL_DB = os.path.expanduser("~/Library/Application Support/com.cxmail.app/cxmail.db")


@unittest.skipUnless(os.path.exists(REAL_DB), "no real CXMail database on this machine")
class RealSchema(unittest.TestCase):
    """The server reads the app's schema directly; fail loudly when a migration
    renames something it uses. Read-only, columns only, no mail."""

    USED = {
        "accounts": {"id", "email", "display_name", "color", "is_active", "sort_order", "hidden_from_aggregates"},
        "folders": {"account_id", "name", "display_name", "folder_type"},
        "messages": {"id", "account_id", "folder_name", "uid", "message_id", "subject", "from_name", "from_email",
                     "to_list", "cc_list", "date", "snippet", "is_read", "is_flagged", "has_attachments",
                     "category", "thread_root_id"},
        "message_bodies": {"id", "account_id", "folder_name", "uid", "plain_text", "sanitized_html"},
        "attachments": {"account_id", "folder_name", "message_uid", "filename", "is_inline"},
        "trusted_image_senders": {"sender_email"},
    }

    def test_columns_exist(self):
        c = sqlite3.connect("file:{}?mode=ro".format(urllib.request.quote(REAL_DB)), uri=True)
        for table, cols in self.USED.items():
            have = {r[1] for r in c.execute("PRAGMA table_info({})".format(table))}
            self.assertFalse(cols - have, "{} is missing {}".format(table, cols - have))
        self.assertTrue(c.execute("SELECT 1 FROM sqlite_master WHERE name = 'messages_fts'").fetchone())
        c.close()


if __name__ == "__main__":
    unittest.main(verbosity=1)
