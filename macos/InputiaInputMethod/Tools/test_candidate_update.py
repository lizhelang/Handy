import importlib.util
from pathlib import Path
import json
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('update_candidate', Path(__file__).resolve().parents[1]/'update-candidate.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CandidateUpdateTests(unittest.TestCase):
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

    def test_symlink_source_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'real').mkdir();(root/'alias').symlink_to(root/'real')
            with self.assertRaises(ValueError):module.canonical(root/'alias')


if __name__ == '__main__':unittest.main()
