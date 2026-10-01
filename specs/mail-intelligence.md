# Mail intelligence

Proactive replies, hybrid semantic/document search, and the Needs You action queue
share local message identities and respect hidden-account/search scope.

## Proactive replies

Per-account opt-in, off by default. A bounded background worker prepares plain-text
suggestions for actionable, unanswered inbound threads, using recipient voice and
pinned rules. Suggestions are stored locally, linked to the message and a snapshot
of its thread. No SMTP or IMAP draft write occurs until the user opens the suggestion
in compose; normal autosave, quote, signature and send behavior then applies.
New replies invalidate suggestions. Human-edited drafts are independent and cannot
be replaced by the worker. Unknown facts remain placeholders. Background payloads
use the existing privacy gate; opted-out accounts never reach inference.

## Search

Local, trigger-maintained document chunks cover cached email bodies and supported
attachments (text, PDF text, DOCX). Extractors have input/output bounds, do not
execute content, and do not write attachment filenames to disk. Image-only PDFs
and legacy binary Office files are unsupported. Older attachments can be indexed
explicitly by fetching their original message; new downloads/parses index locally.

Semantic indexing is a separate per-account opt-in. An OpenAI-compatible embeddings
endpoint supplies real vectors. Default endpoint: local Ollama; alternatively use
the configured OpenAI/compatible AI provider. Vectors are keyed by endpoint and
model so incompatible indices cannot mix; only their fingerprint is stored in SQLite.
Indexing never holds a DB lock across network I/O. A remote endpoint has a daily
cap; local embedding is free and uncapped. Turning meaning search on starts a pass
immediately, and passes work the backlog newest mail first in batches of 32, each
pass time-boxed below the two-minute tick. Settings shows indexed / pending counts
over exactly the chunks the indexer will pick up.
Hidden accounts are excluded from background generation and indexing. Nomic models
receive their documented `search_document` / `search_query` task prefixes.
Disabling an account excludes its vectors.
Keyword and semantic hits share all metadata/account filters. Keyword results remain
available without an embeddings provider. Source-linked answers are explicit,
use only retrieved content, and validate citation ids against actual results.
A citation outside the results is an error; an answer that cites nothing (the
evidence was insufficient) is shown and marked unverified.

## Needs You

Two lanes: Needs my response and Waiting on them. The current classifier and
correspondence gate remain the foundation. A newer sent response clears inbound
work; a newer inbound response clears waiting work. Explicit deadlines and request
text are surfaced from cached mail, with evidence. Ready suggestions can be opened
from a queue item or the reader. Dismissal is persistent and local.

## Verification

Cover fresh/v64 migrations, trigger updates and deletion, account scope (including
empty and hidden scopes), vector validation/model changes, attachment extraction,
reply resolution, stale suggestions, deadline parsing and citation validation.
Frontend checks cover queue lanes, suggestion-to-compose, settings and source links.
Run npm tests/build and cargo test --workspace with the Cargo environment loaded.
