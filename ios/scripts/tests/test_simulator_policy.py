import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / 'simulator_policy.py'
spec = importlib.util.spec_from_file_location('policy', SOURCE)
p = importlib.util.module_from_spec(spec); spec.loader.exec_module(p)
POLICY = p.load(SOURCE.parents[1] / 'simulator-policy.json')
DUO = 'AAAAAAAA-1111-2222-3333-444444444444'
PRO = 'BBBBBBBB-1111-2222-3333-444444444444'
PAD = 'CCCCCCCC-1111-2222-3333-444444444444'


def data():
    return {'devices': {'com.apple.CoreSimulator.SimRuntime.iOS-27-2': [
        {'udid': DUO, 'name':'Shutdown Duo', 'state':'Shutdown', 'isAvailable':True, 'deviceTypeIdentifier':'com.apple.CoreSimulator.SimDeviceType.iPhone-Duo'},
        {'udid': PRO, 'name':'iPhone Duo', 'state':'Booted', 'isAvailable':True, 'deviceTypeIdentifier':'com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro'},
        {'udid': PAD, 'name':'QA', 'isAvailable':True, 'deviceTypeIdentifier':'com.apple.CoreSimulator.SimDeviceType.iPad-Pro-11-inch-M4-8GB'}]}}


class PolicyTests(unittest.TestCase):
    def test_shutdown_duo_is_valid(self):
        self.assertIn('Shutdown Duo', p.validate(POLICY,'compact',DUO.lower(),data()))

    def test_renamed_booted_pro_is_rejected(self):
        with self.assertRaisesRegex(p.PolicyError,'selected iPhone Duo.*iPhone-18-Pro'):
            p.validate(POLICY,'compact',PRO,data())

    def test_wide_requires_ipad(self):
        self.assertIn('wide',p.validate(POLICY,'wide',PAD,data()))
        for profile,uid in [('wide',DUO),('compact',PAD)]:
            with self.assertRaises(p.PolicyError): p.validate(POLICY,profile,uid,data())

    def test_name_booted_missing_unknown_profile_rejected(self):
        for profile,uid in [('compact','booted'),('compact','iPhone Duo'),('compact','DDDDDDDD-1111-2222-3333-444444444444'),('oops',DUO)]:
            with self.assertRaises(p.PolicyError): p.validate(POLICY,profile,uid,data())

    def test_unavailable_wrong_runtime_unknown_type(self):
        for changes in [{'isAvailable':False},{'deviceTypeIdentifier':''}]:
            d=data(); d['devices'][next(iter(d['devices']))][0].update(changes)
            with self.assertRaises(p.PolicyError): p.validate(POLICY,'compact',DUO,d)
        d=data(); d['devices']['com.apple.CoreSimulator.SimRuntime.tvOS-27-2']=d['devices'].pop(next(iter(d['devices'])))
        with self.assertRaises(p.PolicyError): p.validate(POLICY,'compact',DUO,d)

    def test_discovery_and_direct_xcodebuild(self):
        cwd=str(SOURCE.parents[1])
        args=['-project','CXMail.xcodeproj','-destination','platform=iOS Simulator,id='+PRO,'test']
        with self.assertRaises(p.PolicyError): p.check_xcodebuild(args,cwd,data())
        args[3]='platform=iOS Simulator,id='+DUO
        self.assertEqual(len(p.check_xcodebuild(args,cwd,data())),1)
        with self.assertRaises(p.PolicyError): p.check_xcodebuild(['test'],cwd,data())

    def test_mac_build_and_device_tests_unchanged(self):
        cwd=str(SOURCE.parents[1])
        for args in [['-scheme','test','build'],['-destination','platform=iOS,id=physical','test'],['build-for-testing']]:
            with patch.object(p,'inventory',side_effect=AssertionError('must not query')):
                self.assertEqual(p.check_xcodebuild(args,cwd),[])

    def test_multiple_destinations_all_checked(self):
        cwd=str(SOURCE.parents[1])
        with self.assertRaises(p.PolicyError):
            p.check_xcodebuild(['test','-destination','platform=iOS Simulator,id='+DUO,'-destination','platform=iOS Simulator,id='+PRO],cwd,data())

    def test_matrix_preflight_precedes_preferences_and_builds(self):
        script=(SOURCE.parent/'verify.sh').read_text()
        self.assertLess(script.index('scripts/simulator_policy.py'),script.index('defaults read'))
        self.assertLess(script.index('scripts/simulator_policy.py'),script.index('xcodegen generate'))
        self.assertIn('CX_IOS_SIMULATOR_PROFILE="$PROFILE" xcodebuild',script)

    def test_wide_explicit_profile(self):
        with patch.dict(p.os.environ,{'CX_IOS_SIMULATOR_PROFILE':'wide'}):
            self.assertEqual(len(p.check_xcodebuild(['test','-only-testing:CXMailUITests/CXMailUITests/testExactOpenDuoViewport','-destination','platform=iOS Simulator,id='+PAD],str(SOURCE.parents[1]),data())),1)

    def test_id_only_pro_and_mixed_destinations_are_rejected(self):
        cwd=str(SOURCE.parents[1])
        for destinations in [['id='+PRO],['platform=iOS,id=physical','id='+PRO]]:
            args=['test']
            for d in destinations: args += ['-destination',d]
            with self.assertRaises(p.PolicyError): p.check_xcodebuild(args,cwd,data())
        self.assertEqual(len(p.check_xcodebuild(['test','-destination','id='+DUO],cwd,data())),1)

    def test_unknown_id_only_must_not_be_assumed_physical(self):
        with self.assertRaises(p.PolicyError):
            p.check_xcodebuild(['test','-destination','id=unknown'],str(SOURCE.parents[1]),data())

    def test_wide_cannot_run_full_or_unrelated_tests(self):
        for filters in [[],['-only-testing:CXMailUITests/CXMailUITests/testInboxReaderReplyRecovery']]:
            with self.assertRaises(p.PolicyError):
                p.check_xcodebuild(['test','-destination','id='+PAD]+filters,str(SOURCE.parents[1]),data(),environment={'CX_IOS_SIMULATOR_PROFILE':'wide'})
        d=data();d['devices'][next(iter(d['devices']))][2]['deviceTypeIdentifier']='com.apple.CoreSimulator.SimDeviceType.iPad-mini-A17-Pro'
        with self.assertRaises(p.PolicyError): p.validate(POLICY,'wide',PAD,d)

    def test_malformed_profile_returns_actionable_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'policy.json'
            path.write_text(json.dumps({'version':1,'project':'fiction','profiles':{'compact':{}}}))
            with self.assertRaisesRegex(p.PolicyError,'Invalid simulator profile'): p.load(path)


if __name__ == '__main__': unittest.main()
