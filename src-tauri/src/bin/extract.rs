//! cxmail-extract: text out of one attachment, in a process that can't hurt
//! anything.
//!
//! `email::document_text::extract` runs this for PDF and DOCX instead of
//! parsing in-process, launched under `sandbox-exec` with a profile that
//! denies the network, file writes, forking and reads of the user's home
//! folder (see `document_text::SANDBOX_PROFILE`). The parser itself is
//! memory-safe Rust (`pdf-extract` has no `unsafe`, `lopdf` forbids it), so
//! this is not about the buffer-overrun bugs behind PDF CVEs in native
//! renderers; it is about the failures Rust does not prevent. A crafted PDF can
//! loop, or inflate a few megabytes of deflate into gigabytes. Here that costs
//! this process and nothing else:
//!
//! * CPU: `RLIMIT_CPU` of 5 s, plus the parent's wall-clock kill.
//! * Memory: a counting allocator refuses past 512 MB and the process aborts.
//!   macOS does not reliably enforce `RLIMIT_AS`, so the cap lives here.
//! * Input: at most `document_text::MAX_BYTES` on stdin; output capped.
//!
//! Protocol: `cxmail-extract <name> <mime>`, the bytes on stdin, UTF-8 text on
//! stdout, exit 0. Exit 3 = no text. Anything else = failure. Never writes a
//! file and reads nothing but stdin.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

const MEMORY_CAP: usize = 512 * 1024 * 1024;

struct Capped;
static IN_USE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        if IN_USE.fetch_add(size, Ordering::Relaxed) + size > MEMORY_CAP {
            IN_USE.fetch_sub(size, Ordering::Relaxed);
            return std::ptr::null_mut(); // -> handle_alloc_error -> abort
        }
        let p = System.alloc(layout);
        if p.is_null() {
            IN_USE.fetch_sub(size, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static GLOBAL: Capped = Capped;

fn main() {
    let lim = libc::rlimit { rlim_cur: 5, rlim_max: 6 };
    unsafe {
        libc::setrlimit(libc::RLIMIT_CPU, &lim);
    }
    let mut args = std::env::args().skip(1);
    let (Some(name), Some(mime)) = (args.next(), args.next()) else {
        std::process::exit(2);
    };
    let max = cxmail_lib::email::document_text::MAX_BYTES;
    let mut bytes = Vec::new();
    if std::io::stdin().take(max as u64 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > max {
        std::process::exit(2);
    }
    match cxmail_lib::email::document_text::extract_in_process(&name, &mime, &bytes) {
        Some(text) => {
            let mut out = std::io::stdout().lock();
            if out.write_all(text.as_bytes()).is_err() || out.flush().is_err() {
                std::process::exit(1);
            }
        }
        None => std::process::exit(3),
    }
}
