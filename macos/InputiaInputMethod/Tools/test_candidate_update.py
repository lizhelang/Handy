import importlib.util
from pathlib import Path
import json
import tempfile
import unittest
from unittest.mock import patch, call

spec = importlib.util.spec_from_file_location('update_candidate', Path(__file__).resolve().parents[1]/'update-candidate.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CandidateUpdateTests(unittest.TestCase):
    def test_replacement_parent_probe_is_side_effect_bounded(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            target = parent / 'Inputia.app'
            module.assert_replacement_parents([target])
            self.assertEqual(list(parent.iterdir()), [])

    def test_legacy_registration_cleanup_is_bounded_and_failure_is_explicit(self):
        import subprocess
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            old, new, backup = root/'Inputia Candidate.app', root/'Inputia.app', root/'control-before.app'
            old.mkdir(); old.rename(backup); new.mkdir()
            self.assertFalse(old.exists())
            with patch.object(module, 'run', return_value='') as run:
                self.assertTrue(module.unregister_legacy_control(new, new, backup))
                run.assert_not_called()
                self.assertTrue(module.unregister_legacy_control(old, new, backup))
                run.assert_called_once_with(module.REGISTRAR, '-u', backup)
            with patch.object(module, 'run', side_effect=subprocess.CalledProcessError(1, 'lsregister')):
                with patch('builtins.print') as output:
                    self.assertFalse(module.unregister_legacy_control(old, new, backup))
                    output.assert_called_once_with('legacyRegistrationRemoved=false releaseInstalled=true cleanupPending=true', flush=True)

    def test_legacy_registration_cleanup_never_unregisters_new_destination_or_alias(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            old, new, alias = root/'Candidate.app', root/'Inputia.app', root/'backup.app'
            new.mkdir(); alias.symlink_to(new, target_is_directory=True)
            with patch.object(module, 'run') as run, patch('builtins.print'):
                for backup in [new, alias, root/'missing']:
                    self.assertFalse(module.unregister_legacy_control(old, new, backup))
                run.assert_not_called()

    def test_control_path_migrates_only_unique_legacy_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            current, legacy = root/'Inputia.app', root/'Inputia Candidate.app'
            with self.assertRaises(ValueError): module.control_installation(root)
            legacy.mkdir()
            self.assertEqual(module.control_installation(root), (legacy, current))
            current.mkdir()
            with self.assertRaises(ValueError): module.control_installation(root)
            legacy.rmdir()
            self.assertEqual(module.control_installation(root), (current, current))

    def test_control_path_rejects_symlink_even_when_other_install_exists(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root/'Inputia.app').mkdir()
            (root/'Inputia Candidate.app').symlink_to(root/'missing')
            with self.assertRaises(ValueError): module.control_installation(root)

    def test_profile_cannot_change_during_rename(self):
        import plistlib
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory)/'Inputia.app'
            (app/'Contents').mkdir(parents=True)
            info = {'CFBundleIdentifier':'com.pais.handy.UnifiedCandidate',
                    'HandyDevelopmentCandidate':True, 'HandyProfileRunID':'same'}
            path = app/'Contents/Info.plist'
            path.write_bytes(plistlib.dumps(info))
            module.validate_profile(app, 'control', 'same')
            with self.assertRaises(ValueError): module.validate_profile(app, 'control', 'other')
            info['HandyDevelopmentCandidate'] = 'true'
            path.write_bytes(plistlib.dumps(info))
            with self.assertRaises(ValueError): module.validate_profile(app, 'control', 'same')

    def test_release_v2_requires_release_identity_and_rejects_profile_markers(self):
        import plistlib
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory)/'Inputia.app'
            (app/'Contents').mkdir(parents=True)
            path = app/'Contents/Info.plist'
            path.write_bytes(plistlib.dumps({
                'CFBundleIdentifier': 'com.pais.handy.UnifiedCandidate',
                'InputiaReleaseID': 'inputia-1.1.1-85-abc12345',
            }))
            module.validate_release_v2(app, 'control', 'inputia-1.1.1-85-abc12345')
            info = plistlib.loads(path.read_bytes())
            info['HandyDevelopmentCandidate'] = True
            path.write_bytes(plistlib.dumps(info))
            with self.assertRaises(ValueError):
                module.validate_release_v2(app, 'control', 'inputia-1.1.1-85-abc12345')

    def test_release_v2_validates_the_settings_component_identity(self):
        import plistlib
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / 'Inputia 设置.app'
            (app / 'Contents').mkdir(parents=True)
            (app / 'Contents' / 'Info.plist').write_bytes(plistlib.dumps({
                'CFBundleIdentifier': 'com.inputia.settings.UnifiedCandidate',
                'InputiaReleaseID': 'inputia-1.1.1-85-abc12345',
            }))
            module.validate_release_v2(app, 'settings', 'inputia-1.1.1-85-abc12345')
            with (app / 'Contents' / 'Info.plist').open('wb') as stream:
                stream.write(plistlib.dumps({
                    'CFBundleIdentifier': 'com.inputia.inputmethod.Inputia.Settings',
                    'InputiaReleaseID': 'inputia-1.1.1-85-abc12345',
                }))
            with self.assertRaises(ValueError):
                module.validate_release_v2(app, 'settings', 'inputia-1.1.1-85-abc12345')

    def test_rename_failure_restores_original_path_and_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            old, new, ime = root/'Candidate.app', root/'Inputia.app', root/'ime.app'
            for app in [old, ime]:
                app.mkdir(); (app/'value').write_text('old')
            staged = root/'new-control'; staged.mkdir()
            pair, prior, next_pair = root/'pair', root/'prior', root/'next'
            pair.write_text('old'); prior.write_text('old'); next_pair.write_text('new')
            with self.assertRaises(FileNotFoundError):
                module.install_transaction([new, ime], [staged, root/'missing'],
                    [root/'control-backup', root/'ime-backup'], pair, next_pair, prior, [old, ime])
            self.assertTrue(old.is_dir()); self.assertTrue(ime.is_dir())
            self.assertFalse(new.exists()); self.assertEqual(pair.read_text(), 'old')

    def test_post_install_failure_restores_legacy_path_and_preserves_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            old, new, ime = root/'Candidate.app', root/'Inputia.app', root/'ime.app'
            originals, destinations = [old, ime], [new, ime]
            staged, backups = [], []
            for index, app in enumerate(originals):
                app.mkdir(); (app/'value').write_text('old')
                stage = root/f'stage-{index}'; stage.mkdir(); (stage/'value').write_text('new')
                staged.append(stage); backups.append(root/f'backup-{index}')
            pair, prior, next_pair = root/'pair', root/'prior', root/'next'
            pair.write_text('old'); prior.write_text('old'); next_pair.write_text('new')
            module.install_transaction(destinations, staged, backups, pair, next_pair, prior, originals)
            self.assertFalse(old.exists()); self.assertTrue(new.exists())
            module.rollback_installation(destinations, originals, backups, root, pair, prior)
            self.assertTrue(old.exists()); self.assertFalse(new.exists())
            self.assertTrue(all((app/'value').read_text() == 'old' for app in originals+backups))
            self.assertEqual(pair.read_text(), 'old')

    def test_registration_precedes_starting_either_component(self):
        apps = [Path('/Applications/control.app'), Path('/Users/test/Library/Input Methods/ime.app')]
        with patch.object(module, 'run', return_value='') as run:
            module.start_registered_components(apps)
        registrar = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
        self.assertEqual(run.call_args_list, [
            call(registrar, '-f', apps[0]), call(registrar, '-f', apps[1]),
            call('/usr/bin/open', '-a', apps[0], '--args', '--start-hidden'),
            call('/usr/bin/open', '-a', apps[1]),
        ])

    def test_failed_registration_does_not_start_components(self):
        with patch.object(module, 'run', side_effect=RuntimeError('registration failed')) as run:
            with self.assertRaises(RuntimeError):
                module.start_registered_components([Path('/control.app'), Path('/ime.app')])
        self.assertEqual(run.call_count, 1)

    def test_maintenance_needs_fresh_ack_from_both_running_processes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertFalse(module.maintenance_ready(root, [11, 22], now=100))
            for name, pid in [('background',11),('ime',22)]:
                (root/f'permission-health-{name}.json').write_text(json.dumps({'pid':pid,'state':'maintenance','updated_at_ms':99000,'maintenance_marker_epoch':'current'}))
            self.assertTrue(module.maintenance_ready(root, [11,22], 'current', now=100))
            self.assertFalse(module.maintenance_ready(root, [11,22], 'old', now=100))
            self.assertFalse(module.maintenance_ready(root, [11,33], now=100))
            self.assertFalse(module.maintenance_ready(root, [11,22], now=110))
            self.assertFalse(module.maintenance_ready(root, [11,22], now=90))
            (root/'permission-health-ime.json').write_text(json.dumps({'pid':22,'state':'retiring','updated_at_ms':99000}))
            self.assertFalse(module.maintenance_ready(root, [11,22], now=100))

    def test_stopped_component_does_not_block_live_component_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'permission-health-background.json').write_text(json.dumps({'pid':11,'state':'retiring','updated_at_ms':1000}))
            (root/'permission-health-ime.json').write_text(json.dumps({'pid':22,'state':'maintenance','updated_at_ms':99000,'maintenance_marker_epoch':'current'}))
            self.assertTrue(module.maintenance_ready(root, [22], 'current', now=100))
            self.assertFalse(module.maintenance_ready(root, [11,22], 'current', now=100))
            (root/'permission-health-background.json').unlink()
            self.assertTrue(module.maintenance_ready(root, [22], 'current', now=100))
            self.assertFalse(module.maintenance_ready(root, [33], 'current', now=100))

    def test_second_copy_failure_restores_both_apps_and_pair(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            destinations, staged, backups = [], [], []
            for name in ['control','ime']:
                dst=root/name; dst.mkdir(); (dst/'value').write_text('old-'+name)
                destinations.append(dst); staged.append(root/(name+'-new')); backups.append(root/(name+'-before'))
            staged[0].mkdir(); (staged[0]/'value').write_text('new')
            pair=root/'pair'; pair.write_text('old-pair')
            old=root/'old-pair'; old.write_text('old-pair')
            new=root/'new-pair'; new.write_text('new-pair')
            with self.assertRaises(FileNotFoundError):
                module.install_transaction(destinations,staged,backups,pair,new,old)
            self.assertEqual((destinations[0]/'value').read_text(),'old-control')
            self.assertEqual((destinations[1]/'value').read_text(),'old-ime')
            self.assertEqual(pair.read_text(),'old-pair')

    def test_success_replaces_pair_only_after_both_apps(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);dst=[];src=[];back=[]
            for name in ['control','ime']:
                a=root/name;a.mkdir();(a/'value').write_text('old')
                b=root/(name+'-new');b.mkdir();(b/'value').write_text('new')
                dst.append(a);src.append(b);back.append(root/(name+'-old'))
            pair=root/'pair';pair.write_text('old');new=root/'new';new.write_text('new');old=root/'old';old.write_text('old')
            module.install_transaction(dst,src,back,pair,new,old)
            self.assertEqual(pair.read_text(),'new')
            self.assertTrue(all((p/'value').read_text()=='new' for p in dst))
            self.assertTrue(all((p/'value').read_text()=='old' for p in back))

    def test_v2_transaction_replaces_and_rolls_back_all_three_components(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            destinations, staged, backups = [], [], []
            for name in ['control', 'ime', 'settings']:
                destination = root / name
                source = root / f'{name}-new'
                backup = root / f'{name}-old'
                destination.mkdir(); (destination / 'value').write_text('old-' + name)
                source.mkdir(); (source / 'value').write_text('new-' + name)
                destinations.append(destination); staged.append(source); backups.append(backup)
            pair = root / 'pair'; pair.write_text('old-pair')
            old_pair = root / 'pair-before'; old_pair.write_text('old-pair')
            new_pair = root / 'pair-after'; new_pair.write_text('new-pair')

            module.install_transaction(destinations, staged, backups, pair, new_pair, old_pair)
            self.assertEqual(pair.read_text(), 'new-pair')
            self.assertEqual([path.joinpath('value').read_text() for path in destinations],
                             ['new-control', 'new-ime', 'new-settings'])

            failed = root / 'failed'; failed.mkdir()
            module.rollback_installation(destinations, destinations, backups, failed, pair, old_pair)
            self.assertEqual(pair.read_text(), 'old-pair')
            self.assertEqual([path.joinpath('value').read_text() for path in destinations],
                             ['old-control', 'old-ime', 'old-settings'])

    def test_v2_commit_writes_durable_legacy_receipt_and_preserves_installation_id(self):
        import os
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            destinations = [home / 'control.app', home / 'ime.app', home / 'settings.app']
            receipt = module.write_legacy_receipt(
                'trial-20260905', 'inputia-1.1.1-85-test', destinations, home=home)
            self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
            first = json.loads(receipt.read_text())
            self.assertEqual(first['scope'], 'legacy_single_user')
            self.assertEqual(first['data'], {'kind': 'legacy_candidate', 'run_id': 'trial-20260905'})
            self.assertEqual(first['uid'], os.getuid())
            second = module.write_legacy_receipt(
                'trial-20260905', 'inputia-1.1.1-86-test', destinations, home=home)
            self.assertEqual(first['installation_id'], json.loads(second.read_text())['installation_id'])
            self.assertEqual(json.loads(second.read_text())['release_id'], 'inputia-1.1.1-86-test')

    def test_v2_receipt_refuses_foreign_scope_without_overwriting_it(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            path = home / 'Library/Application Support/Inputia/installation.json'
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps({'schema_version': 1, 'scope': 'user'}))
            with self.assertRaises(ValueError):
                module.write_legacy_receipt('trial-20260905', 'inputia-1.1.1-85-test',
                                            [home / 'a', home / 'b', home / 'c'], home=home)
            self.assertEqual(json.loads(path.read_text())['scope'], 'user')

    def test_v2_preflight_accepts_missing_receipt_and_rejects_wrong_component_binding(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            destinations = [home / 'control.app', home / 'ime.app', home / 'settings.app']
            self.assertFalse(module.validate_existing_legacy_receipt(
                'trial-20260905', destinations, home=home))
            module.write_legacy_receipt(
                'trial-20260905', 'inputia-1.1.1-85-test', destinations, home=home)
            self.assertTrue(module.validate_existing_legacy_receipt(
                'trial-20260905', destinations, home=home))
            changed = list(destinations); changed[2] = home / 'other-settings.app'
            with self.assertRaises(ValueError):
                module.validate_existing_legacy_receipt('trial-20260905', changed, home=home)

    def test_v2_preflight_rejects_non_candidate_or_public_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            destinations = [home / 'control.app', home / 'ime.app', home / 'settings.app']
            path = module.write_legacy_receipt(
                'trial-20260905', 'inputia-1.1.1-85-test', destinations, home=home)
            value = json.loads(path.read_text())
            value['channel'] = 'stable'
            path.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                module.validate_existing_legacy_receipt('trial-20260905', destinations, home=home)
            value['channel'] = 'candidate'
            path.write_text(json.dumps(value))
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                module.validate_existing_legacy_receipt('trial-20260905', destinations, home=home)

    def test_symlink_source_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'real').mkdir();(root/'alias').symlink_to(root/'real')
            with self.assertRaises(ValueError):module.canonical(root/'alias')


if __name__ == '__main__':unittest.main()
