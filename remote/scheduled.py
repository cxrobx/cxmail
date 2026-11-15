"""Durable companion schedules. No credentials or message bodies are stored here.

The signed helper re-fetches and checks the reviewed digest at delivery time.
A claimed send is never retried, including after a crash or ambiguous timeout.
This queue is separate from the desktop's outgoing-message queue: phone drafts
stay in IMAP Drafts until the helper sends their exact reviewed version.
"""
import contextlib
import datetime as dt
import os
import sqlite3
import threading
import time


class ScheduleConflict(ValueError):
    pass


class ScheduledQueue:
    def __init__(self, path, sender, on_sent=lambda ref: None):
        self.sender, self.on_sent = sender, on_sent
        self.path = path
        self.operation_lock = threading.RLock()
        os.makedirs(os.path.dirname(os.path.abspath(path)), mode=0o700, exist_ok=True)
        # Private queue metadata (recipients and subject), never the mailbox DB.
        fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
        os.close(fd)
        os.chmod(path, 0o600)
        with self.db() as c:
            c.executescript("""
                CREATE TABLE IF NOT EXISTS sends (
                    id INTEGER PRIMARY KEY, account TEXT NOT NULL,
                    folder TEXT NOT NULL, uid INTEGER NOT NULL, digest TEXT NOT NULL,
                    send_at TEXT NOT NULL, due REAL NOT NULL, subject TEXT NOT NULL,
                    recipients TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending',
                    note TEXT, created REAL NOT NULL);
                CREATE UNIQUE INDEX IF NOT EXISTS one_active_draft
                    ON sends(account, folder, uid) WHERE status IN ('pending','sending');
            """)

    @contextlib.contextmanager
    def db(self):
        c = sqlite3.connect(self.path, timeout=10)
        c.row_factory = sqlite3.Row
        try:
            with c:
                yield c
        finally:
            c.close()

    @staticmethod
    def when(value, now=None):
        try:
            when = dt.datetime.fromisoformat(value.replace('Z', '+00:00'))
            if when.tzinfo is None:
                raise ValueError()
            due = when.timestamp()
        except (ValueError, TypeError, AttributeError, OverflowError):
            raise ValueError('Choose a date and time with a timezone.')
        now = time.time() if now is None else now
        if not now + 30 <= due <= now + 366 * 86400:
            raise ValueError('Choose a time at least a minute from now and within the next year.')
        return when.astimezone(dt.timezone.utc).isoformat(), due

    def active(self, ref):
        with self.db() as c:
            row = c.execute("SELECT * FROM sends WHERE account=? AND folder=? AND uid=? "
                            "AND status IN ('pending','sending')", ref).fetchone()
        return dict(row) if row else None

    def add(self, ref, digest, send_at):
        send_at, due = self.when(send_at)
        with self.operation_lock:
            previous = self.active(ref)
            if previous:
                if previous['digest'] == digest and previous['send_at'] == send_at:
                    return self.public(previous)  # Lost HTTP response: never queue twice.
                raise ScheduleConflict('This draft is already scheduled. Cancel its schedule before changing it.')
            preview = self.sender('preview-draft', *ref)
            if (preview.get('account_id'), preview.get('folder'), preview.get('uid')) != ref:
                raise ValueError('The draft reference changed. Reopen it and review again.')
            if preview.get('digest') != digest:
                raise ValueError('The draft changed after you reviewed it. Reopen it and review again.')
            if not preview.get('sendable'):
                raise ValueError(preview.get('blocked_reason') or 'This draft cannot be sent from the phone.')
            send_at, due = self.when(send_at)  # Preview may have taken two minutes.
            recipients = ', '.join(a['email'] for a in preview.get('to', []))
            with self.db() as c:
                cursor = c.execute("INSERT INTO sends(account,folder,uid,digest,send_at,due,subject,recipients,created) "
                                   "VALUES (?,?,?,?,?,?,?,?,?)",
                                   (*ref, digest, send_at, due, preview.get('subject') or '(no subject)', recipients, time.time()))
                row = c.execute('SELECT * FROM sends WHERE id=?', (cursor.lastrowid,)).fetchone()
            return self.public(row)

    @staticmethod
    def public(row):
        return {k: row[k] for k in ('id', 'account', 'folder', 'uid', 'send_at', 'subject', 'recipients', 'status', 'note')}

    def list(self):
        with self.db() as c:
            # Pending first, then recent outcomes so failures cannot disappear.
            rows = c.execute("SELECT * FROM sends ORDER BY CASE status WHEN 'pending' THEN 0 "
                             "WHEN 'sending' THEN 1 ELSE 2 END, CASE WHEN status='pending' THEN due ELSE -created END LIMIT 200").fetchall()
        return [self.public(row) for row in rows]

    def cancel(self, ident):
        # The conditional UPDATE races safely with the atomic due claim, and
        # must remain responsive while another draft is inside SMTP.
        with self.db() as c:
            changed = c.execute("UPDATE sends SET status='cancelled',note=NULL WHERE id=? AND status='pending'", (ident,)).rowcount
            if not changed:
                raise ScheduleConflict('This schedule is no longer pending. Refresh its status before continuing.')

    def recover(self):
        with self.operation_lock, self.db() as c:
            c.execute("UPDATE sends SET status='uncertain',note=? WHERE status='sending'",
                      ('The mini restarted during delivery. Check Sent before scheduling or sending again. No automatic retry was made.',))

    def run_due(self, now=None):
        """Atomic claim across connections/processes; one attempt per approval."""
        now = time.time() if now is None else now
        while True:
            with self.operation_lock:
                with self.db() as c:
                    c.execute('BEGIN IMMEDIATE')
                    row = c.execute("SELECT * FROM sends WHERE status='pending' AND due<=? ORDER BY due LIMIT 1", (now,)).fetchone()
                    if row is None:
                        return
                    c.execute("UPDATE sends SET status='sending' WHERE id=? AND status='pending'", (row['id'],))
                ref = (row['account'], row['folder'], row['uid'])
                try:
                    out = self.sender('send-draft', *ref, row['digest'])
                    # Record success before the optional in-memory list overlay.
                    status, note = 'sent', out.get('note')
                except Exception as error:
                    status = 'uncertain'
                    note = str(error) + ' Check Sent and Drafts before trying again. No automatic retry was made.'
                with self.db() as c:
                    c.execute('UPDATE sends SET status=?,note=? WHERE id=?', (status, note, row['id']))
                if status == 'sent':
                    self.on_sent(ref)

    def start(self):
        self.recover()
        def work():
            while True:
                try:
                    self.run_due()
                except Exception as error:
                    # A storage error must not kill the resident due scan. Any
                    # already-claimed row remains non-retryable.
                    print('scheduled-send worker: {}'.format(error), flush=True)
                time.sleep(10)
        threading.Thread(target=work, name='scheduled-send', daemon=True).start()
