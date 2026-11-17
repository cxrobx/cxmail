#!/usr/bin/env python3
"""Read-only simulator preflight. Also installed beside the shared iOS hook.

No boots, locks, preference writes or build subprocesses. Policies contain data
only; project code is never imported by the shared hook/guard.
"""
import json
import os
from pathlib import Path
import re
import subprocess
import sys


class PolicyError(ValueError):
    pass


def load(path):
    with open(path) as f:
        policy = json.load(f)
    if policy.get('version') != 1 or not isinstance(policy.get('profiles'), dict):
        raise PolicyError('Unsupported simulator policy: ' + str(path))
    return policy


def discover(cwd, project=None):
    start = Path(cwd).expanduser().resolve()
    if project:
        start = (start / project).resolve().parent
    for parent in (start, *start.parents):
        for candidate in (parent / 'simulator-policy.json', parent / 'ios/simulator-policy.json'):
            if candidate.is_file():
                return candidate
    return None


def inventory():
    try:
        result = subprocess.run(['/usr/bin/xcrun', 'simctl', 'list', 'devices', 'available', '--json'],
                                capture_output=True, text=True, timeout=10, check=True)
        return json.loads(result.stdout)
    except (OSError, ValueError, subprocess.SubprocessError):
        raise PolicyError('Cannot read simulator inventory. No simulator was started. Restore CoreSimulator access and retry; do not guess a device.')


def validate(policy, profile, udid, data):
    if profile not in policy['profiles']:
        raise PolicyError('Unknown simulator profile: ' + str(profile))
    if not isinstance(udid, str) or not re.fullmatch(r'[0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}', udid):
        raise PolicyError('An explicit simulator UUID is required; names and "booted" are ambiguous.')
    matches = [(runtime, device) for runtime, devices in data.get('devices', {}).items()
               for device in devices if device.get('udid', '').lower() == udid.lower()]
    if len(matches) != 1:
        raise PolicyError('Simulator UUID is missing or ambiguous in the available inventory: ' + udid)
    runtime, device = matches[0]
    if not device.get('isAvailable') or not runtime.startswith('com.apple.CoreSimulator.SimRuntime.iOS-'):
        raise PolicyError('Selected simulator is unavailable or is not an iOS runtime.')
    rule = policy['profiles'][profile]
    kind = device.get('deviceTypeIdentifier', '')
    valid = (kind == rule['device_type']) if 'device_type' in rule else (
        bool(rule.get('device_type_prefix')) and kind.startswith(rule['device_type_prefix']))
    if not valid:
        raise PolicyError('{} {} requires {}; selected {} ({})'.format(
            policy['project'], profile, rule['description'], device.get('name', '?'), kind or 'unknown type'))
    return '{}: {} ({}, {})'.format(profile, device.get('name', '?'), kind, runtime)


def values(args, flag):
    return [args[i+1] for i, value in enumerate(args[:-1]) if value == flag] + [
        value.split('=', 1)[1] for value in args if value.startswith(flag + '=')]


def check_xcodebuild(args, cwd=None, data=None):
    # Only actions that actually run tests require simulator geometry. Build and
    # build-for-testing do not run the app. Option values may themselves be "test".
    actions = [value for i, value in enumerate(args) if value in ('test', 'test-without-building')
               and (i == 0 or args[i-1] not in ('-scheme', '-target', '-configuration', '-project', '-workspace'))]
    if not actions:
        return []
    projects = values(args, '-project') or values(args, '-workspace')
    path = discover(cwd or os.getcwd(), projects[0] if projects else None)
    if path is None:
        return []
    destinations = values(args, '-destination')
    simulator = [d for d in destinations if 'platform=iOS Simulator' in d]
    if destinations and not simulator:
        # Real device testing has its own documented install path.
        return []
    if not simulator:
        raise PolicyError('Simulator tests require an explicit -destination platform=iOS Simulator,id=<UUID>.')
    policy = load(path)
    profile = os.environ.get('CX_IOS_SIMULATOR_PROFILE') or policy.get('default_profile', 'compact')
    data = inventory() if data is None else data
    results = []
    for destination in simulator:
        fields = dict(part.strip().split('=', 1) for part in destination.split(',') if '=' in part)
        results.append(validate(policy, profile, fields.get('id'), data))
    return results


def main(args):
    try:
        if args and args[0] == '--xcodebuild':
            results = check_xcodebuild(args[1:])
        else:
            if len(args) < 2:
                raise PolicyError('Usage: simulator_policy.py <policy.json> <profile=UUID> [...]')
            policy = load(args[0]); data = inventory(); results = []
            for pair in args[1:]:
                profile, sep, udid = pair.partition('=')
                if not sep:
                    raise PolicyError('Expected profile=UUID')
                results.append(validate(policy, profile, udid, data))
        for result in results:
            print('iOS simulator preflight: ' + result, file=sys.stderr)
        return 0
    except (PolicyError, OSError, ValueError) as error:
        print('iOS simulator preflight BLOCKED: ' + str(error), file=sys.stderr)
        return 64


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
