#!/usr/bin/env python3
"""Fiction-only checks for shared MCP/iOS draft-link configuration."""
import json
import runpy
import tempfile
import unittest
from pathlib import Path

MODULE = runpy.run_path(str(Path(__file__).with_name('configure-draft-links.py')))


class DraftLinkConfiguration(unittest.TestCase):
    def test_one_origin_configures_both_clients(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            MODULE['configure']('https://links.example.com/', root / 'data', root / 'ios')
            self.assertEqual(json.loads((root / 'data/draft-links.json').read_text()),
                             {'base_url': 'https://links.example.com'})
            self.assertEqual((root / 'ios/Config/Links.xcconfig').read_text(),
                             'CXMAIL_LINK_HOST = links.example.com\n')

    def test_refuses_credentials_ports_paths_and_transport_changes(self):
        for origin in ['http://links.example.com', 'https://user:secret@links.example.com',
                       'https://links.example.com:443', 'https://links.example.com/path',
                       'https://links.example.com?token=bad', 'https://links.example.com#send',
                       'https://links..example.com', 'https://localhost']:
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                MODULE['link_host'](origin)


if __name__ == '__main__':
    unittest.main()
