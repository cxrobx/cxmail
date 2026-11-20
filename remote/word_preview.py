"""Bounded, read-only Word preview using the installed T3 Code renderer."""

from io import BytesIO
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import threading
from zipfile import BadZipFile, ZipFile

RENDERER = Path(__file__).with_name("word-preview") / "render.cjs"
RENDER_SLOT = threading.BoundedSemaphore(1)
MIB = 1024 * 1024
FORMATS = {
    "word": (".docx", "word/document.xml", "document.worker-BJ3XBWBI.js"),
    "xlsx": (".xlsx", "xl/workbook.xml", "spreadsheet.worker-AXWWY4sf.js"),
    "pptx": (".pptx", "ppt/presentation.xml", "presentation.worker-o5xvygYW.js"),
}


def ready(kind):
    home = Path.home()
    asset_roots = (
        home / "Library/Application Support/com.cxmail.remote/word-preview/0.1.7-office/assets",
        home / "Library/Application Support/com.cxmail.remote/word-preview/0.1.7/assets",
        home / ".t3/runtime/versions/0.1.7/client/assets",
    )
    return (RENDERER.parent / "node_modules/playwright-core").is_dir() and any(
        (root / FORMATS[kind][2]).is_file() for root in asset_roots
    )


def acceptable_package(source, required):
    if len(source) > 5 * MIB:
        return False
    try:
        with ZipFile(BytesIO(source)) as package:
            entries = package.infolist()
            if len(entries) > 1024 or sum(e.file_size for e in entries) > 32 * MIB:
                return False
            if len({e.filename for e in entries}) != len(entries):
                return False
            if required not in package.namelist():
                return False
            for entry in entries:
                if entry.file_size > 4 * MIB or entry.flag_bits & 1:
                    return False
                if entry.filename.lower().endswith((".xml", ".rels")):
                    xml = package.read(entry)
                    if re.search(br"<!\s*(?:DOCTYPE|ENTITY)\b", xml.replace(b"\x00", b""), re.I):
                        return False
                    if xml.count(b"<") > 100_000:
                        return False
    except (OSError, RuntimeError, BadZipFile, NotImplementedError, ValueError):
        return False
    return True


def render_word(source):
    return render_office(source, "word")


def render_office(source, kind):
    node = shutil.which("node") or "/opt/homebrew/bin/node"
    if kind not in FORMATS or not ready(kind) or not acceptable_package(source, FORMATS[kind][1]) or not RENDER_SLOT.acquire(blocking=False):
        return None
    try:
        with subprocess.Popen(
            [node, str(RENDERER), kind], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, start_new_session=True,
        ) as process:
            try:
                pdf, _ = process.communicate(source, timeout=38)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()
                return None
            if process.returncode == 0 and 0 < len(pdf) <= 25 * MIB and pdf.startswith(b"%PDF-"):
                return pdf
    except OSError:
        return None
    finally:
        RENDER_SLOT.release()
    return None
