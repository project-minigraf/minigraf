"""Synthetic controls only: no benchmark, provider, or network calls."""
import copy
from decimal import Decimal, localcontext
import importlib.util
import json
import math
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent


def load_module(name):
    path = ROOT / (name + '.py')
    if not path.exists():
        return None
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


decide = load_module('decide')
BASE = [98, 99, 100, 100, 101, 102]
CASE = 'prepared_execute/1k'


def sets(base=None, variant=None):
    base = BASE[:] if base is None else base
    return {'B1': base[:], 'B2': base[:],
            'V': [2 * x for x in base] if variant is None else variant}


def attempts(values):
    return [{'run_index': i, 'attempt': 1, 'valid': True, 'reason': '',
             'value_ns': v, 'sample_means_ns': [v] * 10}
            for i, v in enumerate(values, 1)]


def document():
    configs = []
    for kind in ('physical', 'simulated'):
        configs.append({
            'id': 'synthetic-' + kind, 'platform': 'ios', 'kind': kind,
            'reference': None if kind == 'physical' else 'synthetic-physical',
            'blocking_reason': None,
            'build': {'core_revision': '1' * 40, 'binding_revision': '2' * 40,
                      'harness_revision': '3' * 40, 'profile': 'Release',
                      'slowdown_flag': 'repeat-timed-operation-twice',
                      'artifact_sha256': {s: ('4' if kind == 'physical' else '5') * 64
                                          for s in ('B1', 'B2', 'V')}},
            'runtime': {'description': 'SYNTHETIC ONLY; not a real runtime'},
            'cases': {case: {'blocking_reason': None,
                             'sets': {s: attempts(v) for s, v in sets().items()}}
                      for case in decide.CATEGORIES}})
    return {'schema_version': 1, 'synthetic': True, 'configurations': configs}


class Controls(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(decide, 'decision tooling has not been implemented')

    def assert_rejected(self, physical, simulated, failed):
        result = decide.evaluate_pair(physical, simulated)
        self.assertEqual(result['outcome'], 'rejected', result)
        self.assertEqual([k for k, v in result['checks'].items() if not v],
                         [failed], result)

    def test_all_pass_and_fractional_delta(self):
        result = decide.evaluate_pair(sets(), sets())
        self.assertEqual(result['outcome'], 'qualified')
        self.assertEqual(result['metrics']['physical']['delta'], 1.0)
        self.assertEqual(result['metrics']['simulated']['delta'], 1.0)

    def test_closeness_low(self):
        self.assert_rejected(sets(), sets([x * .66 for x in BASE]), 'closeness')

    def test_closeness_high(self):
        self.assert_rejected(sets(), sets([x * 1.51 for x in BASE]), 'closeness')

    def test_closeness_inclusive_endpoints(self):
        for ratio in (.67, 1.5):
            self.assertEqual(decide.evaluate_pair(sets(), sets([x * ratio for x in BASE]))
                             ['outcome'], 'qualified')

    def test_cv_absolute(self):
        # Physical CV 10%, simulated CV 16%; ratio still below 2.
        self.assert_rejected(sets([85, 95, 100, 100, 105, 115]),
                             sets([76, 92, 100, 100, 108, 124]),
                             'cv_absolute')

    def test_cv_inclusive_boundaries(self):
        physical = sets([85, 95, 100, 100, 105, 115])
        simulated = sets([77.5, 92.5, 100, 100, 107.5, 122.5])
        self.assertEqual(decide.evaluate_pair(physical, simulated)['outcome'], 'qualified')
        simulated = sets([100 + 2*(x-100) for x in BASE])
        self.assertEqual(decide.evaluate_pair(sets(), simulated)['outcome'], 'qualified')

    def test_cv_relative(self):
        self.assert_rejected(sets(), sets([95, 97.5, 100, 100, 102.5, 105]),
                             'cv_relative')

    def test_sample_cv_convention(self):
        values = [80, 90, 100, 110, 120]
        cv = decide.summarize(values)['cv']
        self.assertAlmostEqual(cv, math.sqrt(250) / 100)
        self.assertGreater(cv, .15)
        self.assertLess(statistics.pstdev(values) / statistics.mean(values), .15)
        # Five runs demonstrate the convention, but cannot meet registered N=6.
        self.assertEqual(decide.evaluate_pair(sets(values), sets(values))['outcome'],
                         'inconclusive')

    def test_no_detection_simulated(self):
        self.assert_rejected(sets(), sets(variant=[1, 2, 199, 201, 400, 500]),
                             'detection_simulated')

    def test_no_detection_physical(self):
        self.assert_rejected(sets(variant=[1, 2, 199, 201, 400, 500]), sets(),
                             'detection_physical')

    def test_delta_mismatch(self):
        self.assert_rejected(sets(), sets(variant=[3 * x for x in BASE]),
                             'delta_agreement')

    def test_delta_agreement_inclusive_endpoints(self):
        for multiplier in (1.5, 2.5):
            self.assertEqual(decide.evaluate_pair(sets(), sets(variant=[multiplier * x for x in BASE]))
                             ['outcome'], 'qualified')

    def test_aa_each_direction_each_platform(self):
        for platform in ('physical', 'simulated'):
            for scale, direction in ((1.2, 'b2_gt_b1'), (.8, 'b1_gt_b2')):
                with self.subTest(platform=platform, direction=direction):
                    p, s = sets(), sets()
                    target = p if platform == 'physical' else s
                    target['B2'] = [scale * x for x in BASE]
                    self.assert_rejected(p, s, 'aa_' + platform + '_' + direction)

    def test_degenerate_every_set(self):
        for platform in ('physical', 'simulated'):
            for group in ('B1', 'B2', 'V'):
                for bad in ([100] * 6, [0] + BASE[1:], [-1] + BASE[1:],
                            [float('nan')] + BASE[1:], [float('inf')] + BASE[1:],
                            [None] + BASE[1:], [True] + BASE[1:], BASE[:5], [],
                            [1e308, 1e308, 9e307, 8e307, 7e307, 6e307]):
                    with self.subTest(platform=platform, group=group, bad=bad):
                        p, s = sets(), sets()
                        (p if platform == 'physical' else s)[group] = bad
                        self.assertEqual(decide.evaluate_pair(p, s)['outcome'], 'inconclusive')

    def test_nonpositive_physical_delta_overrides_failure(self):
        for variant in (BASE[:], [x * .9 for x in BASE]):
            result = decide.evaluate_pair(sets(variant=variant), sets([2*x for x in BASE]))
            self.assertEqual(result['outcome'], 'inconclusive')

    def test_student_t_known_and_extreme_tails(self):
        self.assertAlmostEqual(decide.student_t_sf(2, 10), .0366940173853702, delta=1e-12)
        self.assertAlmostEqual(decide.student_t_sf(0, 10), .5, delta=1e-15)
        self.assertAlmostEqual(decide.student_t_sf(1, 1), .25, delta=1e-14)
        self.assertAlmostEqual(decide.student_t_sf(2, 2), .09175170953613698, delta=1e-14)
        for t in (.01, 2, 100, 1e6):
            # Analytic df=1 Cauchy survival, stable for large t.
            expected = math.atan(1/t) / math.pi
            self.assertAlmostEqual(decide.student_t_sf(t, 1) / expected, 1, delta=1e-11)
        self.assertAlmostEqual(decide.student_t_sf(100, 10) / 1.2248447777099239e-16,
                               1, delta=1e-11)
        self.assertAlmostEqual(decide.student_t_sf(-2, 10), 1 - .0366940173853702, delta=1e-12)
        self.assertGreater(decide.student_t_sf(1e100, 1), 0)
        self.assertAlmostEqual(decide.student_t_sf(1e200, 1) / (1e-200 / math.pi),
                               1, delta=1e-11)
        for args in ((float('nan'), 10), (1, 0), (1, float('inf'))):
            with self.assertRaises(ValueError):
                decide.student_t_sf(*args)

    def test_student_t_near_zero(self):
        density_zero = math.exp(math.lgamma(5.5) - math.lgamma(5)) / math.sqrt(10 * math.pi)
        for t in (1e-10, 1e-9, 1e-8, 1e-7):
            self.assertAlmostEqual(decide.student_t_sf(t, 10), .5 - density_zero*t,
                                   delta=1e-14)

    def test_student_t_independent_even_df_integral(self):
        # At df=10, substitute s=t/sqrt(10+t²) and integrate (1-s²)^4.
        # Decimal avoids cancellation in this independent polynomial oracle.
        with localcontext() as context:
            context.prec = 80
            for text in ('0.001', '0.1', '1', '2', '4.14370049404659', '10', '100'):
                t = Decimal(text)
                s = t / (10 + t*t).sqrt()
                integral = sum(Decimal(c) * s**(2*k+1) / (2*k+1)
                               for k, c in enumerate((1, -4, 6, -4, 1)))
                expected = float(Decimal('.5') - Decimal(315)/256 * integral)
                self.assertAlmostEqual(decide.student_t_sf(float(t), 10) / expected,
                                       1, delta=1e-10)

    def test_beta_known_values(self):
        self.assertAlmostEqual(decide.regularized_beta(.5, 2, 3), .6875, delta=1e-13)
        self.assertAlmostEqual(decide.regularized_beta(.2, .5, .5),
                               2 * math.asin(math.sqrt(.2)) / math.pi, delta=1e-13)

    def test_welch_unequal_variance_degrees_freedom(self):
        a, b = [1, 2, 3], [1, 3, 5, 7]
        result = decide.welch_greater(a, b)
        self.assertAlmostEqual(result['t'], -math.sqrt(2))
        self.assertAlmostEqual(result['df'], 216/53)  # 4 / (1/18 + 25/27)
        self.assertGreater(result['p'], .5)
        reverse = decide.welch_greater(b, a)
        self.assertAlmostEqual(result['p'] + reverse['p'], 1)

    def test_retry_retained_not_counted(self):
        doc = document()
        runs = doc['configurations'][1]['cases'][CASE]['sets']['B1']
        runs[0]['attempt'] = 2
        runs.insert(0, {'run_index': 1, 'attempt': 1, 'valid': False,
                        'reason': 'provider failure', 'value_ns': None})
        before = copy.deepcopy(doc)
        report = decide.decide(doc)
        row = next(r for r in report['comparisons'] if r['case'] == CASE)
        self.assertEqual(row['outcome'], 'qualified')
        self.assertEqual(row['audit']['simulated']['B1'],
                         {'attempts': 7, 'invalid_attempts': 1, 'valid_runs': 6})
        self.assertEqual(doc, before)

    def test_invalid_retry_patterns_are_inconclusive(self):
        for change in ('after_valid', 'duplicate', 'gap', 'cap', 'bad_invalid_value',
                       'missing_reason', 'out_of_range', 'wrong_median'):
            with self.subTest(change=change):
                doc = document()
                runs = doc['configurations'][1]['cases'][CASE]['sets']['B1']
                if change in ('after_valid', 'duplicate'):
                    extra = copy.deepcopy(runs[0])
                    if change == 'after_valid':
                        extra['attempt'] = 2
                    runs.insert(1, extra)
                elif change in ('gap', 'cap'):
                    runs[0]['attempt'] = 2 if change == 'gap' else 4
                elif change == 'bad_invalid_value':
                    runs[0]['valid'] = False
                    runs[0]['reason'] = 'validation failure'
                elif change == 'missing_reason':
                    runs[0].update(valid=False, value_ns=None)
                    runs[0].pop('sample_means_ns')
                elif change == 'out_of_range':
                    runs[0]['run_index'] = 7
                else:
                    runs[0]['sample_means_ns'][0:6] = [1000] * 6
                row = next(r for r in decide.decide(doc)['comparisons'] if r['case'] == CASE)
                self.assertEqual(row['outcome'], 'inconclusive', row)

    def test_exhausted_retries_are_inconclusive(self):
        doc = document()
        runs = doc['configurations'][1]['cases'][CASE]['sets']['V']
        runs[:] = runs[1:] + [dict(run_index=1, attempt=n, valid=False,
                                  reason='infrastructure failure', value_ns=None)
                              for n in (1, 2, 3)]
        row = next(r for r in decide.decide(doc)['comparisons'] if r['case'] == CASE)
        self.assertEqual(row['outcome'], 'inconclusive')
        self.assertEqual(row['audit']['simulated']['V']['valid_runs'], 5)

    def test_category_cannot_override_case(self):
        doc = document()
        doc['configurations'][1]['cases'][CASE]['sets'] = {
            s: attempts(v) for s, v in sets([x*2 for x in BASE]).items()}
        report = decide.decide(doc)
        self.assertEqual(len(report['comparisons']), 14)
        self.assertEqual(sum(r['outcome'] == 'qualified' for r in report['comparisons']), 13)
        self.assertEqual(next(r for r in report['comparisons'] if r['case'] == CASE)
                         ['outcome'], 'rejected')
        self.assertEqual(report['categories']['synthetic-simulated']
                         [decide.CATEGORIES[CASE]]['rejected'], 1)

    def test_unmatched_unavailable_and_missing_case(self):
        for change in ('build', 'profile', 'artifact', 'runtime', 'device', 'case', 'missing'):
            with self.subTest(change=change):
                doc = document()
                sim = doc['configurations'][1]
                if change == 'build':
                    sim['build']['core_revision'] = 'f'*40
                elif change == 'profile':
                    sim['build']['profile'] = 'Debug'
                elif change == 'artifact':
                    sim['build']['artifact_sha256']['V'] = 'TODO'
                elif change == 'runtime':
                    sim['runtime'] = {}
                elif change == 'device':
                    sim['blocking_reason'] = 'device unavailable'
                elif change == 'case':
                    sim['cases'][CASE]['blocking_reason'] = 'timed operations cannot match'
                else:
                    del sim['cases'][CASE]
                row = next(r for r in decide.decide(doc)['comparisons'] if r['case'] == CASE)
                self.assertEqual(row['outcome'], 'inconclusive')

    def test_android_and_multiple_candidates(self):
        doc = document()
        android = copy.deepcopy(doc['configurations'])
        for config in android:
            config['platform'] = 'android'
            config['id'] += '-android'
            if config['reference']:
                config['reference'] += '-android'
        extra = copy.deepcopy(android[1])
        extra['id'] += '-candidate2'
        doc['configurations'] += android + [extra]
        report = decide.decide(doc)
        self.assertEqual(len(report['comparisons']), 42)
        self.assertTrue(all(r['outcome'] == 'qualified' for r in report['comparisons']))

    def test_value_only_input_and_bad_numbers(self):
        doc = document()
        for config in doc['configurations']:
            for case in config['cases'].values():
                for runs in case['sets'].values():
                    for run in runs:
                        del run['sample_means_ns']
        self.assertTrue(all(r['outcome'] == 'qualified'
                            for r in decide.decide(doc)['comparisons']))
        for bad in (float('nan'), float('inf'), 10**1000):
            doc['configurations'][1]['cases'][CASE]['sets']['B1'][0]['value_ns'] = bad
            row = next(r for r in decide.decide(doc)['comparisons'] if r['case'] == CASE)
            self.assertEqual(row['outcome'], 'inconclusive')
            decide.render_json(decide.decide(doc))

    def test_duplicate_json_keys_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'raw.json'
            path.write_text('{"schema_version": 1, "schema_version": 1}')
            with self.assertRaises(ValueError):
                decide.load_raw(path)

    def test_schema_errors_fail_closed(self):
        for change in ('duplicate_id', 'bad_reference', 'wrong_platform', 'extra_case', 'version', 'boolean_version'):
            doc = document()
            if change == 'duplicate_id':
                doc['configurations'].append(copy.deepcopy(doc['configurations'][0]))
            elif change == 'bad_reference':
                doc['configurations'][1]['reference'] = 'missing'
            elif change == 'wrong_platform':
                doc['configurations'][1]['platform'] = 'android'
            elif change == 'extra_case':
                doc['configurations'][1]['cases']['unknown'] = {}
            else:
                doc['schema_version'] = True if change == 'boolean_version' else 2
            with self.subTest(change=change), self.assertRaises(ValueError):
                decide.decide(doc)

    def test_deterministic_fixture_and_reproduction(self):
        raw = ROOT / 'fixtures' / 'synthetic.json'
        self.assertTrue(raw.exists(), 'synthetic fixture has not been created')
        doc = json.loads(raw.read_text())
        report = decide.decide(doc)
        self.assertEqual(decide.render_json(report).encode(),
                         (ROOT / 'fixtures' / 'synthetic-report.json').read_bytes())
        self.assertEqual(decide.render_markdown(report).encode(),
                         (ROOT / 'fixtures' / 'synthetic-report.md').read_bytes())
        doc['configurations'].reverse()
        self.assertEqual(decide.render_json(report), decide.render_json(decide.decide(doc)))
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'report'
            command = [sys.executable, '-I', str(ROOT / 'decide.py'), str(raw), '--output', str(out)]
            subprocess.run(command, check=True, capture_output=True)
            self.assertEqual(out.with_suffix('.json').read_bytes(), decide.render_json(report).encode())
            command = [sys.executable, '-I', str(ROOT / 'reproduce.py'), str(raw), str(out)]
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
            out.with_suffix('.json').write_text('{}\n')
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
            out.with_suffix('.json').write_text(decide.render_json(report))
            out.with_suffix('.md').write_text('corrupted\n')
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
            raw_dir = Path(tmp) / 'raw'
            raw_dir.mkdir()
            (raw_dir / 'raw-samples.json').write_bytes(raw.read_bytes())
            out.with_suffix('.md').write_text(decide.render_markdown(report))
            command[3] = str(raw_dir)
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)


if __name__ == '__main__':
    unittest.main()
