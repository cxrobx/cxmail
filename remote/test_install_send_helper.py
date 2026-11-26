"""remote/install-send-helper.sh refuses a phone helper that can't renew Gmail (gotcha #70)."""
import os
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "install-send-helper.sh")


def fake_helper(path, google):
    with open(path, "w") as f:
        f.write('#!/bin/sh\n[ "$2" = build-info ] && echo \'{"google_oauth": %s}\'\n' % ("true" if google else "false"))
    os.chmod(path, 0o755)


class InstallSendHelperTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = self.tmp.name
        self.dest = os.path.join(self.dir, "cxmail-remote-send")
        self.new = os.path.join(self.dir, "cxmail-mcp")

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, dest):
        env = dict(os.environ, HOME=self.dir, CXMAIL_SEND_BIN=dest)
        return subprocess.run(["/bin/bash", SCRIPT, self.new], env=env, capture_output=True, text=True)

    def test_refuses_a_build_without_the_google_client(self):
        with open(self.dest, "w") as f:
            f.write("old")
        fake_helper(self.new, google=False)
        p = self.run_script(self.dest)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("no Google sign-in client", p.stderr)
        self.assertEqual(open(self.dest).read(), "old")
        self.assertFalse([d for d in os.listdir(self.dir) if d.startswith("rollback-send-")])

    def test_refuses_a_build_signed_unlike_the_current_helper(self):
        fake_helper(self.dest, google=True)  # unsigned, so its requirement can't be read
        before = open(self.dest).read()
        fake_helper(self.new, google=True)
        p = self.run_script(self.dest)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("REFUSED", p.stderr)
        self.assertEqual(open(self.dest).read(), before)

    def test_installs_a_good_build_where_there_was_none(self):
        fake_helper(self.new, google=True)
        p = self.run_script(self.dest)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(open(self.dest).read(), open(self.new).read())

    def test_nothing_to_do_without_a_separate_helper(self):
        fake_helper(self.new, google=False)
        p = self.run_script("")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("Nothing to do", p.stdout)


if __name__ == "__main__":
    unittest.main()
