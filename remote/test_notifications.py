import contextlib
import datetime
import os
import sqlite3
import tempfile
import unittest
import uuid
from notifications import Notifications, important

class NotificationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.mailpath = os.path.join(self.tmp.name, 'mail.sqlite')
        c = sqlite3.connect(self.mailpath)
        c.executescript('''CREATE TABLE accounts(id TEXT, email TEXT, is_active INTEGER, hidden_from_aggregates INTEGER);
            INSERT INTO accounts VALUES ('a','work@example.org',1,0), ('b','personal@example.org',1,0);
            CREATE TABLE folders(account_id TEXT,name TEXT,folder_type TEXT,uidvalidity INTEGER);
            INSERT INTO folders VALUES ('a','INBOX','inbox',1);
            INSERT INTO folders VALUES ('b','INBOX','inbox',1);
            CREATE TABLE messages(id INTEGER PRIMARY KEY,account_id TEXT,folder_name TEXT,uid INTEGER,
                message_id TEXT,from_email TEXT,is_read INTEGER,date TEXT,flags TEXT,category TEXT DEFAULT 'updates',
                from_name TEXT DEFAULT 'Jordan', subject TEXT DEFAULT 'Tuesday plan', snippet TEXT DEFAULT 'Can we meet?',
                thread_root_id TEXT);
            CREATE TABLE message_bodies(account_id TEXT,folder_name TEXT,uid INTEGER,plain_text TEXT);
            CREATE TABLE triage_verdicts(account_id TEXT,folder_name TEXT,uid INTEGER,needs_response REAL,
                needs_action REAL,category TEXT,urgency INTEGER,model TEXT,prompt_version TEXT,withheld_reason TEXT);
            CREATE TABLE message_risk(account_id TEXT,message_key TEXT,p_phishing REAL,p_impersonation REAL);
        ''')
        c.commit(); c.close()
        self.calls = []
        self.result = (200, '')
        def sender(*args):
            self.calls.append(args)
            if isinstance(self.result, Exception): raise self.result
            return self.result
        self.worker = Notifications(self.mailbox, os.path.join(self.tmp.name, 'push.sqlite'), sender)
        self.worker.set_account({'account':'a','enabled':True})
        self.device = {'device': str(uuid.uuid4()), 'token': 'ab' * 32, 'environment': 'development', 'enabled': True}
        self.worker.trust({'email':'trusted@example.org','trusted':True})

    def tearDown(self): self.tmp.cleanup()

    @contextlib.contextmanager
    def mailbox(self):
        c = sqlite3.connect(self.mailpath); c.row_factory = sqlite3.Row
        try:
            with c: yield c
        finally: c.close()

    def mail(self, uid=1, sender='trusted@example.org', date=None, account='a', thread=None):
        date = date or datetime.datetime.now(datetime.timezone.utc).isoformat()
        with self.mailbox() as c:
            c.execute('INSERT INTO messages(id,account_id,folder_name,uid,message_id,from_email,is_read,date,flags,thread_root_id) VALUES (?,?,?,?,?,?,?,?,?,?)',
                      (uid, account,'INBOX',uid,'<m{}@example.org>'.format(uid),sender,0,date,'',thread))
            c.execute("INSERT INTO message_bodies VALUES (?,'INBOX',?,'fictional body')", (account, uid))

    def verdict(self, uid=1, category='primary', urgency=1, model='decisions:gpt-6-luna', withheld=None):
        with self.mailbox() as c:
            c.execute('INSERT INTO triage_verdicts VALUES (?,?,?,?,?,?,?,?,?,?)',
                      ('a','INBOX',uid,.2,.01,category,urgency,model,'v2',withheld))

    def test_new_mail_only_and_restart_dedupe(self):
        self.mail(1); self.worker.register(self.device); self.mail(2)
        self.worker.tick(); self.worker.tick()
        other = Notifications(self.mailbox, self.worker.path, self.worker.sender); other.tick()
        self.assertEqual(len(self.calls),1)
        self.assertEqual(self.calls[0][2]['uid'],2)
        self.assertNotIn('trusted@example.org', str(self.calls[0][2]))
        self.assertEqual(self.calls[0][2]['aps']['alert'],
                         {'title':'Jordan','subtitle':'Tuesday plan','body':'Can we meet?'})

    def test_account_scope_precedes_trust_and_inference_and_never_backfills(self):
        self.worker.register(self.device)
        classified = []
        self.worker.classifier = lambda row: classified.append(row['uid']) or {}
        self.mail(1, account='b')
        self.mail(2, account='b', sender='unknown@example.org')
        self.worker.tick()
        self.assertEqual((self.calls, classified), ([], []))
        self.worker.set_account({'account':'b','enabled':True})
        self.mail(3, account='b')
        self.worker.tick()
        self.assertEqual([call[2]['uid'] for call in self.calls], [3])
        self.worker.set_account({'account':'b','enabled':False})
        self.mail(4, account='b')
        self.worker.tick()
        self.assertEqual([call[2]['uid'] for call in self.calls], [3])

    def watch(self, uid, enabled=True, account='b'):
        return self.worker.watch({'account': account, 'folder': 'INBOX', 'uid': uid, 'enabled': enabled})

    def test_watched_thread_notifies_every_later_reply_without_inference(self):
        # Account b has notifications off and nothing is important or trusted.
        self.worker.register(self.device)
        classified = []
        self.worker.classifier = lambda row: classified.append(row['uid']) or {}
        self.mail(1, account='b', sender='unknown@example.org', thread='<root@example.org>')
        self.mail(2, account='b', sender='unknown@example.org', thread='<root@example.org>')
        self.assertEqual(self.watch(1), {'ok': True, 'notify': True})
        self.assertIn(('b', '<root@example.org>'), self.worker.watched())
        self.worker.tick()
        self.assertEqual(self.calls, [], 'replies before Notify Me are never backfilled')
        self.mail(3, account='b', sender='unknown@example.org', thread='<root@example.org>')
        self.mail(4, account='b', sender='unknown@example.org', thread='<other@example.org>')
        self.mail(5, account='b', sender='personal@example.org', thread='<root@example.org>')
        self.worker.tick()
        self.assertEqual([call[2]['uid'] for call in self.calls], [3])
        self.assertEqual(classified, [], 'a watched reply skips inference; other mail stays out of scope')

    def test_single_message_thread_matches_by_message_id(self):
        self.worker.register(self.device)
        self.mail(1, account='b', sender='unknown@example.org')
        self.watch(1)
        self.mail(2, account='b', sender='unknown@example.org', thread='<m1@example.org>')
        self.worker.tick()
        self.assertEqual([call[2]['uid'] for call in self.calls], [2])

    def test_watched_thread_still_blocks_junk_and_phishing_and_stops_when_unwatched(self):
        self.worker.register(self.device)
        self.mail(1, account='b', sender='unknown@example.org', thread='<root@example.org>')
        self.watch(1)
        for uid in (2, 3, 4):
            self.mail(uid, account='b', sender='unknown@example.org', thread='<root@example.org>')
        with self.mailbox() as c:
            c.execute("UPDATE messages SET flags='$Junk' WHERE uid=2")
            c.execute("INSERT INTO message_risk VALUES ('b','m3@example.org',.9,0)")
            c.execute("INSERT INTO triage_verdicts VALUES ('b','INBOX',4,0,0,'junk',0,'decisions:gpt-6-luna','v2',NULL)")
        self.worker.tick()
        self.assertEqual(self.calls, [])
        self.assertEqual(self.watch(1, enabled=False), {'ok': True, 'notify': False})
        self.mail(5, account='b', sender='unknown@example.org', thread='<root@example.org>')
        self.worker.tick()
        self.assertEqual(self.calls, [])

    def test_watch_rejects_bad_input_and_unknown_mail(self):
        for bad in ({}, {'account': 'b', 'folder': 'INBOX', 'uid': '1', 'enabled': True},
                    {'account': 'b', 'folder': 'INBOX', 'uid': 1, 'enabled': 'yes'}):
            with self.assertRaises(ValueError): self.worker.watch(bad)
        with self.assertRaises(ValueError): self.watch(99)

    def test_account_disabled_during_inference_prevents_delivery(self):
        self.worker.register(self.device)
        self.mail(sender='unknown@example.org')
        def classify(_):
            self.worker.set_account({'account':'a','enabled':False})
            return {'model':'decisions:gpt-6-luna','category':'primary',
                    'needs_response':1,'needs_action':0,'urgency':3}
        self.worker.classifier = classify
        self.worker.tick()
        self.assertEqual(self.calls, [])

    def test_smart_waits_for_verdict(self):
        self.worker.register(self.device); self.mail(sender='other@example.org')
        self.worker.tick(); self.assertEqual(self.calls,[])
        self.verdict(); self.worker.tick(); self.assertEqual(len(self.calls),1)

    def test_junk_read_hidden_old_withheld_risky_are_suppressed(self):
        self.worker.register(self.device)
        for i in range(1,8): self.mail(i)
        self.verdict(1,category='junk'); self.verdict(2,withheld='privacy')
        with self.mailbox() as c: c.execute("UPDATE messages SET from_email='other@example.org' WHERE uid=2")
        with self.mailbox() as c:
            c.execute('UPDATE messages SET is_read=1 WHERE uid=3')
            c.execute("UPDATE messages SET date='2000-01-01' WHERE uid=4")
            c.execute("UPDATE messages SET flags='$Junk' WHERE uid=5")
            c.execute("INSERT INTO message_risk VALUES ('a','m6@example.org',.9,.01)")
        self.worker.tick(); self.assertEqual([r[2]['uid'] for r in self.calls],[7])
        self.calls.clear()
        self.mail(8)
        with self.mailbox() as c: c.execute('UPDATE accounts SET hidden_from_aggregates=1')
        self.worker.tick(); self.assertEqual(self.calls,[])

    def test_promotions_and_chat_model_do_not_trigger(self):
        self.worker.register(self.device)
        self.mail(1,'other@example.org'); self.verdict(1,category='promotions')
        self.mail(2,'other@example.org'); self.verdict(2,model='gpt-6-luna')
        self.worker.tick(); self.assertEqual(self.calls,[])
        self.mail(3,'other@example.org'); self.verdict(3,category='updates',urgency=4)
        self.worker.tick(); self.assertEqual(len(self.calls),1)

    def test_ambiguous_failure_is_not_retried(self):
        self.worker.register(self.device); self.mail(); self.result = OSError('timeout')
        self.worker.tick(); self.worker.tick(); self.assertEqual(len(self.calls),1)

    def test_definite_transient_failure_retries(self):
        self.worker.register(self.device); self.mail(); self.result = (503,'ServiceUnavailable')
        self.worker.tick(); self.worker.tick(); self.assertEqual(len(self.calls),1)
        with self.worker.connect() as c: c.execute('UPDATE deliveries SET retry_at=0')
        self.result = (200,''); self.worker.tick(); self.worker.tick(); self.assertEqual(len(self.calls),2)

    def test_dead_token_disabled_and_rotation_keeps_watermark(self):
        self.worker.register(self.device); self.mail(); self.result=(410,'Unregistered'); self.worker.tick()
        self.mail(2); self.worker.tick(); self.assertEqual(len(self.calls),1)
        self.worker.register(dict(self.device,token='cd'*32)); self.mail(3)
        self.result=(200,''); self.worker.tick(); self.assertEqual(self.calls[-1][2]['uid'],3)

    def test_disabled_and_reenabled_no_backfill(self):
        self.worker.register(self.device); self.mail()
        self.worker.register(dict(self.device, enabled=False)); self.worker.tick(); self.assertEqual(self.calls,[])
        self.worker.register(self.device); self.worker.tick(); self.assertEqual(self.calls,[])
        self.mail(2); self.worker.tick(); self.assertEqual(len(self.calls),1)

    def test_on_demand_classification_cached_and_budgeted(self):
        calls=[]
        def classify(r):
            calls.append(r['uid'])
            return dict(needs_response=.2, needs_action=.01, category='primary',urgency=1,model='decisions:gpt-6-luna')
        self.worker.classifier=classify
        self.worker.register(self.device)
        self.mail(sender='other@example.org')
        self.worker.tick(); self.worker.tick()
        self.assertEqual(calls,[1]); self.assertEqual(len(self.calls),1)
        self.worker.register(dict(self.device,token='cd'*32)); self.worker.tick()
        self.assertEqual(len(self.calls),1)

    def test_missing_body_pending_and_provider_failure_not_rebilled(self):
        calls=[]
        def classify(r):
            calls.append(r['uid'])
            return {'pending': True}
        self.worker.classifier=classify; self.worker.register(self.device)
        self.mail(sender='other@example.org'); self.worker.tick(); self.worker.tick()
        self.assertEqual(calls,[1,1]); self.assertEqual(len(self.calls),0)
        def failed(r):
            calls.append(r['uid']); raise ValueError('provider')
        self.worker.classifier=failed; self.worker.tick(); self.worker.tick()
        self.assertEqual(calls,[1,1,1])

    def test_disable_during_inference_prevents_delivery(self):
        def classify(r):
            self.worker.disable(self.device)
            return dict(needs_response=.5, needs_action=0, category='primary',urgency=3,model='decisions:gpt-6-luna')
        self.worker.classifier=classify; self.worker.register(self.device)
        self.mail(sender='other@example.org'); self.worker.tick(); self.assertEqual(self.calls,[])

    def test_cached_negative_is_not_reclassified(self):
        calls=[]
        self.worker.classifier=lambda r:calls.append(r)
        self.worker.register(self.device)
        self.mail(sender='other@example.org'); self.verdict(category='promotions')
        self.worker.tick(); self.assertEqual(calls,[]); self.assertEqual(self.calls,[])

    def test_daily_cap_survives_restart(self):
        calls=[]
        self.worker.classifier=lambda r:calls.append(r)
        self.worker.register(self.device); self.mail(sender='other@example.org')
        import time
        with self.worker.connect() as c:
            c.executemany('INSERT INTO verdicts VALUES (?,NULL,?)', [('key'+str(i),time.time()) for i in range(400)])
        self.worker.tick(); self.assertEqual(calls,[])

    def test_read_during_inference_prevents_push(self):
        def classify(r):
            with self.mailbox() as c: c.execute('UPDATE messages SET is_read=1')
            return dict(needs_response=.5,needs_action=0,category='primary',urgency=3,model='decisions:gpt-6-luna')
        self.worker.classifier=classify; self.worker.register(self.device)
        self.mail(sender='other@example.org'); self.worker.tick(); self.assertEqual(self.calls,[])

    def test_explicit_trust_not_blocked_by_inference_privacy_withholding(self):
        calls=[]
        self.worker.classifier=lambda r:calls.append(r)
        self.worker.register(self.device); self.mail(); self.verdict(withheld='financial sender')
        self.worker.tick()
        self.assertEqual(calls,[]); self.assertEqual(len(self.calls),1)

    def test_invalid_registration_and_trust(self):
        for field,value in [('device','oops'),('token','bad'),('environment','evil.example'),('enabled',1)]:
            with self.assertRaises(ValueError): self.worker.register(dict(self.device,**{field:value}))
        for email in ['Name <a@example.org>','*@example.org','a@example.org\n']:
            # wildcard is not a sender address
            with self.assertRaises(ValueError): self.worker.trust({'email':email,'trusted':True})
        with self.assertRaises(ValueError): self.worker.trust({'email':'a@example.org','trusted':'yes'})

if __name__ == '__main__': unittest.main()
