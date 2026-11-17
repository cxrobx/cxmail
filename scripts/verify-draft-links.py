#!/usr/bin/env python3
"""Check the exact association variants iOS and command-line clients request.

A successful uncompressed CDN response does not prove native iOS can open a
universal link: Apple's CDN can cache an older gzip error separately.
"""
import argparse
import gzip
import json
import sys
import urllib.error
import urllib.request

import runpy
from pathlib import Path
link_host = runpy.run_path(str(Path(__file__).with_name("configure-draft-links.py")))["link_host"]


def verify(base: str, app_id: str) -> None:
    host = link_host(base)
    endpoints = {
        "Origin": f"https://{host}/.well-known/apple-app-site-association",
        "Apple CDN": f"https://app-site-association.cdn-apple.com/a/v1/{host}",
    }
    for label, url in endpoints.items():
        for encoding in ("identity", "gzip"):
            request = urllib.request.Request(url, headers={"Accept-Encoding": encoding,
                                                          "User-Agent": "CXMail-Association-Check/1.0"})
            try:
                with urllib.request.urlopen(request, timeout=25) as response:
                    if response.url != url:
                        raise ValueError("association must not redirect")
                    body = response.read(131_073)
                    if len(body) > 131_072:
                        raise ValueError("association exceeds Apple's size limit")
                    if response.headers.get("Content-Encoding") == "gzip":
                        body = gzip.decompress(body)
                    details = json.loads(body)["applinks"]["details"]
                    matched = any(app_id in item.get("appIDs", []) and
                                  any(c.get("/") == "/draft/*" and not c.get("exclude")
                                      for c in item.get("components", [])) for item in details)
                    if not matched:
                        raise ValueError("app identifier or /draft/* association does not match")
                print(f"{label} ({encoding}): valid")
            except urllib.error.HTTPError as error:
                reason = error.headers.get("Apple-Failure-Reason", "association unavailable")
                raise SystemExit(f"{label} ({encoding}): HTTP {error.code}: {reason}") from None


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_url")
    parser.add_argument("--app-id", required=True, help="Signed application identifier: team prefix plus bundle id.")
    args = parser.parse_args()
    verify(args.base_url, args.app_id)
