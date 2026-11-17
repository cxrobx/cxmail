"""Durable native push outbox. Mailbox connections remain read-only.

The existing, privacy-gated Decisions triage pass owns inference. This consumer
never sends mail to another model or treats image permission as notification trust.
"""
import hashlib
import json
import math
import os
import re
import sqlite3
import subprocess
import threading
import time
import uuid
from contextlib import contextmanager

STATE = os.path.expanduser('~/Library/Application Support/com.cxmail.remote')


def important(row, trusted):
    # Trust is an explicit exact-address preference, never a display-name match.
    if (row['is_read'] or row['category'] == 'junk'
            or '$junk' in (row['flags'] or '').lower()
            or (row['p_phishing'] or 0) >= .5 or (row['p_impersonation'] or 0) >= .5):
        return False
    if (row['from_email'] or '').strip().lower() in trusted:
        return True
    if row['withheld_reason']:
        return False
    if row['model'] != 'decisions:gpt-6-luna' or row['prompt_version'] != 'v2':
        return False
    values = [row['needs_response'], row['needs_action'], row['urgency']]
    if any(not isinstance(v, (int, float)) or not math.isfinite(v) for v in values):
        return False
    # Start conservatively: material deadline OR a primary message needing us.
    return (row['urgency'] >= 3 or (row['category'] == 'primary' and
            (row['needs_response'] >= .1 or row['needs_action'] >= .7)))


class Notifications:
    def __init__(self, mailbox, path=None, sender=None, classifier=None):
        self.mailbox = mailbox
        self.path = path or os.path.join(STATE, 'notifications.sqlite')
        self.sender = sender or self.send
        self.classifier = classifier
        self.lock = threading.RLock()
        os.makedirs(os.path.dirname(self.path), mode=0o700, exist_ok=True)
        with self.connect() as c:
            c.executescript('''
                CREATE TABLE IF NOT EXISTS devices (
                    id TEXT PRIMARY KEY, token TEXT NOT NULL, environment TEXT NOT NULL,
                    enabled INTEGER NOT NULL, start_id INTEGER NOT NULL, updated REAL NOT NULL);
                CREATE TABLE IF NOT EXISTS trusted (email TEXT PRIMARY KEY);
                CREATE TABLE IF NOT EXISTS verdicts (
                    message TEXT PRIMARY KEY, result TEXT, attempted REAL NOT NULL);
                CREATE TABLE IF NOT EXISTS deliveries (
                    device TEXT NOT NULL, message TEXT NOT NULL, state TEXT NOT NULL,
                    attempts INTEGER NOT NULL, retry_at REAL NOT NULL, updated REAL NOT NULL,
                    PRIMARY KEY(device, message));
            ''')
        os.chmod(self.path, 0o600)

    @contextmanager
    def connect(self):
        c = sqlite3.connect(self.path, timeout=10)
        c.row_factory = sqlite3.Row
        try:
            with c:
                yield c
        finally:
            c.close()

    def configured(self):
        return all(os.path.isfile(p) for p in (
            os.path.join(STATE, 'apns.json'), os.path.join(STATE, 'cxmail-apns')))

    def settings(self):
        with self.connect() as c:
            return {'configured': self.configured(), 'trusted_senders': [r[0] for r in c.execute(
                'SELECT email FROM trusted ORDER BY email')]}

    def trust(self, data):
        email = data.get('email')
        if not isinstance(email, str) or '*' in email or not re.fullmatch(r'[^\s@<>]{1,100}@[^\s@<>]{1,150}\.[^\s@<>]{1,50}', email):
            raise ValueError('Use an exact sender email address')
        if type(data.get('trusted')) is not bool:
            raise ValueError('trusted must be a boolean')
        with self.connect() as c:
            if data['trusted']:
                c.execute('INSERT OR IGNORE INTO trusted VALUES (?)', (email.lower(),))
            else:
                c.execute('DELETE FROM trusted WHERE email=?', (email.lower(),))
        return self.settings()

    def disable(self, data):
        try:
            ident = str(uuid.UUID(data.get('device')))
        except (ValueError, TypeError, AttributeError):
            raise ValueError('Invalid device identity')
        with self.connect() as c:
            c.execute('UPDATE devices SET enabled=0 WHERE id=?', (ident,))
        return self.settings()

    def register(self, data):
        ident, token, env, enabled = (data.get(k) for k in ('device', 'token', 'environment', 'enabled'))
        try:
            ident = str(uuid.UUID(ident))
        except (ValueError, TypeError, AttributeError):
            raise ValueError('Invalid device identity')
        if not isinstance(token, str) or not re.fullmatch('[0-9a-f]{32,512}', token):
            raise ValueError('Invalid APNs token')
        if env not in ('development', 'production') or type(enabled) is not bool:
            raise ValueError('Invalid push environment or enabled flag')
        with self.lock, self.mailbox() as m, self.connect() as c:
            start = m.execute('SELECT coalesce(max(id),0) FROM messages').fetchone()[0]
            old = c.execute('SELECT * FROM devices WHERE id=?', (ident,)).fetchone()
            if old and old['enabled'] and enabled:
                start = old['start_id']
            c.execute('INSERT OR REPLACE INTO devices VALUES (?,?,?,?,?,?)',
                      (ident, token, env, enabled, start, time.time()))
        return self.settings()

    def tick(self):
        if not self.configured() and self.sender == self.send:
            return
        with self.connect() as c, self.mailbox() as m:
            classifications = 0
            deliveries = 0
            trusted = {r[0] for r in c.execute('SELECT email FROM trusted')}
            devices = c.execute('SELECT * FROM devices WHERE enabled=1').fetchall()
            for d in devices:
                # Never backfill pre-registration mail, old sync imports, hidden accounts,
                # read mail, or non-inbox folders. Pending triage is reconsidered next tick.
                rows = m.execute('''SELECT m.*, v.needs_response, v.needs_action,
                        v.category AS triage_category, v.urgency, v.model, v.prompt_version, v.withheld_reason,
                        risk.p_phishing, risk.p_impersonation,
                        EXISTS(SELECT 1 FROM message_bodies b WHERE b.account_id=m.account_id
                            AND b.folder_name=m.folder_name AND b.uid=m.uid
                            AND trim(coalesce(b.plain_text,''))<>'') AS body_available
                    FROM messages m JOIN accounts a ON a.id=m.account_id
                    JOIN folders f ON f.account_id=m.account_id AND f.name=m.folder_name
                    LEFT JOIN triage_verdicts v ON v.account_id=m.account_id
                        AND v.folder_name=m.folder_name AND v.uid=m.uid
                    LEFT JOIN message_risk risk ON risk.account_id=m.account_id
                        AND risk.message_key=CASE WHEN trim(coalesce(m.message_id,'')) <> ''
                            THEN lower(trim(trim(m.message_id), '<>'))
                            ELSE 'uid:' || m.folder_name || ':' || m.uid END
                    WHERE m.id>? AND a.is_active=1 AND a.hidden_from_aggregates=0
                        AND f.folder_type='inbox' AND m.is_read=0
                        AND julianday(m.date)>julianday('now','-1 day')
                    ORDER BY m.id DESC LIMIT 2000''', (d['start_id'],)).fetchall()
                for r in rows:
                    active = c.execute('SELECT enabled,start_id FROM devices WHERE id=?', (d['id'],)).fetchone()
                    if not active or not active['enabled']:
                        break
                    if r['id'] <= active['start_id']:
                        continue
                    trusted = {t[0] for t in c.execute('SELECT email FROM trusted')}
                    r = dict(r)
                    if r.get('category') == 'junk':
                        continue
                    r['category'] = r['triage_category']
                    # Known hard exclusions precede inference as well as delivery.
                    if (r['category'] == 'junk' or '$junk' in (r['flags'] or '').lower()
                            or (r['p_phishing'] or 0) >= .5 or (r['p_impersonation'] or 0) >= .5):
                        continue
                    if r['withheld_reason'] and (r['from_email'] or '').strip().lower() not in trusted:
                        continue
                    # Message-ID dedupes copies and moves; mailbox generation guards UID reuse.
                    mid = (r['message_id'] or '').strip().strip('<>').lower()
                    if not mid:
                        generation = m.execute('SELECT uidvalidity FROM folders WHERE account_id=? AND name=?',
                                               (r['account_id'], r['folder_name'])).fetchone()[0]
                        mid = '{}:{}:{}'.format(r['folder_name'], generation, r['uid'])
                    key = hashlib.sha256((r['account_id'] + ':' + mid).encode()).hexdigest()
                    if (not important(r, trusted) and self.classifier
                            and (r['model'] != 'decisions:gpt-6-luna' or r['prompt_version'] != 'v2')):
                        if not r['body_available']:
                            continue
                        cached = c.execute('SELECT result FROM verdicts WHERE message=?', (key,)).fetchone()
                        if cached:
                            verdict = json.loads(cached[0]) if cached[0] else {}
                        elif classifications < 5 and c.execute('SELECT count(*) FROM verdicts WHERE attempted>?', (time.time()-86400,)).fetchone()[0] < 400:
                            # Record a possible billed attempt before calling. A crash or provider
                            # error cannot repeatedly bill the same email after restart.
                            classifications += 1
                            c.execute('INSERT INTO verdicts VALUES (?,NULL,?)', (key,time.time()))
                            c.commit()
                            try:
                                verdict = self.classifier(r)
                                if not isinstance(verdict, dict):
                                    raise ValueError('Invalid verdict')
                                if verdict.get('pending'):
                                    c.execute('DELETE FROM verdicts WHERE message=?', (key,))
                                else:
                                    c.execute('UPDATE verdicts SET result=? WHERE message=?', (json.dumps(verdict),key))
                                c.commit()
                            except (OSError, ValueError, subprocess.TimeoutExpired):
                                continue
                        else:
                            continue
                        if verdict.get('withheld'):
                            continue
                        for field in ('needs_response','needs_action','category','urgency','model'):
                            r[field] = verdict.get(field)
                        r['prompt_version'] = 'v2'
                    if not important(r, trusted):
                        continue
                    old = c.execute('SELECT * FROM deliveries WHERE device=? AND message=?', (d['id'], key)).fetchone()
                    if old and (old['state'] != 'retry' or old['retry_at'] > time.time() or old['attempts'] >= 5):
                        continue
                    current = c.execute('SELECT * FROM devices WHERE id=?', (d['id'],)).fetchone()
                    if not current or not current['enabled'] or r['id'] <= current['start_id']:
                        continue
                    d = current
                    live = m.execute('''SELECT m.is_read,a.is_active,a.hidden_from_aggregates,f.folder_type
                        FROM messages m JOIN accounts a ON a.id=m.account_id
                        JOIN folders f ON f.account_id=m.account_id AND f.name=m.folder_name
                        WHERE m.id=?''', (r['id'],)).fetchone()
                    if not live or live['is_read'] or not live['is_active'] or live['hidden_from_aggregates'] or live['folder_type'] != 'inbox':
                        continue
                    # Recheck trust revocation after slow inference.
                    fresh_trust = {t[0] for t in c.execute('SELECT email FROM trusted')}
                    if not important(r, fresh_trust):
                        continue
                    if deliveries >= 5:
                        break
                    deliveries += 1
                    attempts = old['attempts'] + 1 if old else 1
                    # Persist before the network call: crash/ambiguous timeout cannot double-send.
                    c.execute('INSERT OR REPLACE INTO deliveries VALUES (?,?,?,?,?,?)',
                              (d['id'], key, 'uncertain', attempts, 0, time.time()))
                    c.commit()
                    payload = {'aps': {'alert': {'title': 'CXMail', 'body': 'New important email'},
                                       'sound': 'default', 'thread-id': 'cxmail'},
                               'account': r['account_id'], 'folder': r['folder_name'], 'uid': r['uid']}
                    try:
                        status, reason = self.sender(d, key, payload)
                    except (OSError, ValueError, subprocess.TimeoutExpired):
                        continue
                    state = 'sent' if status == 200 else 'rejected'
                    if status == 429 or status >= 500:
                        state = 'retry'
                    if status == 410 or reason in ('BadDeviceToken', 'DeviceTokenNotForTopic', 'Unregistered'):
                        c.execute('UPDATE devices SET enabled=0 WHERE id=?', (d['id'],))
                    c.execute('UPDATE deliveries SET state=?,retry_at=?,updated=? WHERE device=? AND message=?',
                              (state, time.time() + min(3600, 30 * 2 ** attempts), time.time(), d['id'], key))
                    c.commit()
                    if state == 'rejected':
                        break
            c.execute('DELETE FROM deliveries WHERE updated<?', (time.time() - 7 * 86400,))
            c.execute('DELETE FROM verdicts WHERE attempted<?', (time.time() - 7 * 86400,))

    def send(self, device, key, payload):
        p = subprocess.run([os.path.join(STATE, 'cxmail-apns'), os.path.join(STATE, 'apns.json')],
            input=json.dumps({'token': device['token'], 'environment': device['environment'],
                              'collapse': key, 'payload': payload}),
            text=True, capture_output=True, timeout=25)
        if p.returncode:
            raise ValueError('APNs helper unavailable')
        result = json.loads(p.stdout)
        return int(result['status']), result.get('reason', '')

    def start(self):
        def run():
            while True:
                try:
                    self.tick()
                except Exception:
                    # Never log mail, tokens, credentials, or response bodies.
                    import sys
                    sys.stderr.write('CXMail notification pass unavailable\n')
                time.sleep(30)
        threading.Thread(target=run, daemon=True).start()
