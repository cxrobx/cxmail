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
    if not isinstance(policy, dict):
        raise PolicyError('Simulator policy must be a JSON object: ' + str(path))
    if policy.get('version') != 1 or not isinstance(policy.get('profiles'), dict):
        raise PolicyError('Unsupported simulator policy: ' + str(path))
    if not isinstance(policy.get('project'), str) or not policy['profiles']:
        raise PolicyError('Simulator policy requires a project and profiles')
    for name, rule in policy['profiles'].items():
        if not isinstance(rule, dict) or not isinstance(rule.get('description'), str) or not any(
                isinstance(rule.get(k), str) and rule[k] for k in ('device_type', 'device_type_prefix')):
            raise PolicyError('Invalid simulator profile: ' + str(name))
    for field in ('context','references'):
        if field in policy and (not isinstance(policy[field],list) or any(not isinstance(x,str) for x in policy[field])):
            raise PolicyError(field + ' must be a list of strings')
    if policy.get('default_profile','compact') not in policy['profiles']:
        raise PolicyError('default_profile is not a defined profile')
    for name,rule in policy['profiles'].items():
        if 'only_testing' in rule and (not isinstance(rule['only_testing'],list) or not rule['only_testing'] or
                                      any(not isinstance(x,str) or not x for x in rule['only_testing'])):
            raise PolicyError('only_testing must be a nonempty list of test identifiers: ' + name)
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


def check_xcodebuild(args, cwd=None, data=None, environment=None):
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
    if not destinations:
        raise PolicyError('Simulator tests require an explicit -destination with a UUID.')
    policy = load(path)
    environment = os.environ if environment is None else environment
    profile = environment.get('CX_IOS_SIMULATOR_PROFILE') or policy.get('default_profile', 'compact')
    rule = policy['profiles'].get(profile)
    if rule is None:
        raise PolicyError('Unknown simulator profile: ' + str(profile))
    if rule.get('only_testing'):
        selected = values(args, '-only-testing') + [a.split(':',1)[1] for a in args if a.startswith('-only-testing:')]
        allowed = rule['only_testing']
        if not selected or any(not any(item == a or item.startswith(a + '/') for a in allowed) for item in selected):
            raise PolicyError('Wide profile requires the documented -only-testing fixture subset; full/unrelated suites require the compact Duo.')
    results = []
    for destination in destinations:
        fields = {k.strip():v.strip() for k,v in (part.split('=',1) for part in destination.split(',') if '=' in part)}
        platform = fields.get('platform')
        udid = fields.get('id')
        data = inventory() if data is None else data
        known = udid and any(d.get('udid','').lower() == udid.lower()
                             for ds in data.get('devices',{}).values() for d in ds)
        # Inventory identity wins over a contradictory/misspelled platform.
        if known:
            results.append(validate(policy,profile,udid,data))
            continue
        if platform in ('iOS','macOS','tvOS','watchOS','visionOS'):
            continue
        if not udid:
            raise PolicyError('Simulator tests require an explicit simulator UUID, not a name or generic destination.')
        raise PolicyError('Destination is not a known simulator; physical devices must explicitly specify platform=iOS.')

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
