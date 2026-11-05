import json
from pathlib import Path
import tempfile
import unittest

from run_variants import copy_pair_fixtures


class PairFixtureTests(unittest.TestCase):
    def test_pairs_have_identical_bytes_but_independent_files_and_wals(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            source.mkdir()
            for suffix, data in [('.graph', b'database'), ('.graph.wal', b'pending'), ('.json', b'{}')]:
                (source / ('fixture' + suffix)).write_bytes(data)
            copies = copy_pair_fixtures(source, root / 'pairs', ['fixture'])
            for suffix in ['.graph', '.graph.wal', '.json']:
                left, right = [copies[role] / ('fixture' + suffix) for role in ['baseline', 'control']]
                self.assertEqual(left.read_bytes(), right.read_bytes())
                self.assertNotEqual(left.stat().st_ino, right.stat().st_ino)
                self.assertNotEqual(left.stat().st_ino, (source / left.name).stat().st_ino)
            (copies['control'] / 'fixture.graph.wal').write_bytes(b'changed')
            self.assertEqual((copies['baseline'] / 'fixture.graph.wal').read_bytes(), b'pending')
            hashes = json.loads((root / 'pairs' / 'hashes.json').read_text())
            self.assertEqual(set(hashes), {'fixture.graph', 'fixture.graph.wal', 'fixture.json'})
