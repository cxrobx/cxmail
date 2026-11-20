#!/usr/bin/env python3
"""Keep the pinned renderer available when T3 prunes old installed versions."""

from pathlib import Path
import re
import shutil
import tempfile

VERSION = "0.1.7"
CLIENT = Path.home() / f".t3/runtime/versions/{VERSION}/client"
SOURCE = CLIENT / "assets"
DEST = Path.home() / f"Library/Application Support/com.cxmail.remote/word-preview/{VERSION}-office"
ENTRIES = (
    "document.worker-BJ3XBWBI.js", "spreadsheet.worker-AXWWY4sf.js",
    "presentation.worker-o5xvygYW.js", "worker-rpc-CoD-MTl9.js",
)
REFERENCE = re.compile(r"(?:/assets/|\./)([\w.-]+\.(?:js|wasm|ttf|woff2))")


def stage():
    if (DEST / "assets" / ENTRIES[0]).is_file():
        return
    pending = list(ENTRIES)
    copied = set()
    DEST.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=DEST.parent) as working:
        release = Path(working) / VERSION
        target = release / "assets"
        target.mkdir(parents=True)
        while pending:
            name = pending.pop()
            if name in copied:
                continue
            asset = SOURCE / name
            shutil.copyfile(asset, target / name)
            copied.add(name)
            if asset.suffix == ".js":
                pending.extend(n for n in REFERENCE.findall(asset.read_text()) if (SOURCE / n).is_file())
        for notice in SOURCE.glob("LICENSE*"):
            if notice.is_file():
                shutil.copyfile(notice, target / notice.name)
        shutil.copyfile(CLIENT / "third-party-licenses.json", release / "third-party-licenses.json")
        release.rename(DEST)
    print(f"Staged {len(copied)} Office renderer assets from T3 Code {VERSION}.")


if __name__ == "__main__":
    stage()
