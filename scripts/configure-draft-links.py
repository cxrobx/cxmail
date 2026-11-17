#!/usr/bin/env python3
"""Configure the public draft-link origin for MCP handoffs and the iOS build.
Deploy and verify the metadata host before running this or installing the app.
No mailbox endpoint, credentials or email content belongs in this config.
"""
import argparse
import json
import re
from pathlib import Path
from urllib.parse import urlsplit


def link_host(base):
    url = urlsplit(base)
    host = url.hostname or ""
    if (url.scheme != "https" or url.netloc != host or url.path not in ("", "/")
            or url.query or url.fragment or not re.fullmatch(r"[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?\.[a-z]{2,}", host)
            or ".." in host or len(host) > 253):
        raise ValueError("Use an HTTPS origin with a lowercase DNS hostname and no credentials, port, path or query")
    return host


def configure(base, data_dir, ios_dir):
    host = link_host(base)
    data_dir.mkdir(parents=True, exist_ok=True)
    (data_dir / "draft-links.json").write_text(json.dumps({"base_url": "https://" + host}) + "\n")
    config = ios_dir / "Config"
    config.mkdir(parents=True, exist_ok=True)
    (config / "Links.xcconfig").write_text("CXMAIL_LINK_HOST = " + host + "\n")
    print("Configured MCP handoffs and iOS association host: " + host)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_url")
    parser.add_argument("--data-dir", type=Path, default=Path.home() / "Library/Application Support/com.cxmail.app")
    args = parser.parse_args()
    configure(args.base_url, args.data_dir, Path(__file__).resolve().parents[1] / "ios")
