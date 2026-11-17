# Simulator preflight

`../simulator-policy.json` is the machine-independent contract. Compact tests
require an actual iPhone Duo, and the wide fixture requires the 11-inch M4 iPad Pro used by the harness. The
preflight checks `deviceTypeIdentifier` and the iOS runtime, not the name or
whether the device happens to be booted. It enumerates all available simulators,
including id-only xcodebuild destinations.

`verify.sh` validates BOTH UUIDs before changing preferences, generating the
project, or starting a build. Bad/missing/ambiguous/unavailable devices or an
unreadable inventory fail immediately. No tests have run when preflight fails.

```
/usr/bin/python3 scripts/simulator_policy.py simulator-policy.json compact=<UUID> wide=<UUID>
/usr/bin/python3 -m unittest discover -s scripts/tests
```

Chris's shared xcodebuild guard uses a copy of this same validator for direct
and nested simulator test commands. Direct tests default to compact; use
`CX_IOS_SIMULATOR_PROFILE=wide` for the documented -only-testing fixture subset.
An unfiltered full suite cannot use wide to bypass the compact Duo requirement.
The wrapper validates only; it never pauses an already-running script for a
context refresh. The agent hook owns the reminder before script execution. Existing
resource limits still apply after device validation. The machine's execution
reminder is documented in `~/.claude/docs/ios-execution-context.md`; the portable
repository preflight works even without those hooks installed.

When updating this module on the maintainer machine, install the same bytes to
`~/.claude/hooks/ios_simulator_policy.py` and run its hook tests as well.
