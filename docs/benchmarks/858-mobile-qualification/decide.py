#!/usr/bin/env python3
"""Offline AC12 qualification. All inference uses independent run values."""
import argparse
import json
import math
from pathlib import Path
import re
import statistics
import sys

N = 6
SETS = ('B1', 'B2', 'V')
CATEGORIES = {
    'prepared_execute/1k': 'allocation/conversion',
    'cursor_batch_1/1k': 'allocation/conversion',
    'cursor_batch_256/1k': 'allocation/conversion',
    'set_roundtrip/2': 'allocation/conversion',
    'predicate/1k': 'compute/callback',
    'aggregate/1k': 'compute/callback',
    'prepare/point': 'compute/callback',
    'register_predicate/first': 'compute/callback',
    'register_aggregate/first': 'compute/callback',
    'fact_log_batch_256/1k': 'allocation/conversion',
    'options_error/negative_cache': 'allocation/conversion',
    'options_readonly_open/1k': 'storage/startup',
    'file_transact/1k': 'storage/startup',
    'log_writer_append_finish/1k': 'storage/startup',
}


def positive(value):
    try:
        return (type(value) in (int, float) and math.isfinite(value) and value > 0)
    except OverflowError:
        return False


def _beta_fraction(a, b, x):
    """Modified Lentz continued fraction for incomplete beta."""
    tiny = 1e-300

    def bounded(v):
        return v if abs(v) >= tiny else math.copysign(tiny, v)

    c = 1.0
    d = 1 / bounded(1 - (a + b) * x / (a + 1))
    h = d
    for m in range(1, 10001):
        for term in (m * (b - m) * x / ((a + 2*m - 1) * (a + 2*m)),
                     -(a + m) * (a + b + m) * x / ((a + 2*m) * (a + 2*m + 1))):
            d = 1 / bounded(1 + term * d)
            c = bounded(1 + term / c)
            step = d * c
            h *= step
        if abs(step - 1) < 3e-15:
            return h
    raise ValueError('incomplete beta did not converge')


def regularized_beta(x, a, b):
    if not (positive(a) and positive(b) and math.isfinite(x) and 0 <= x <= 1):
        raise ValueError('invalid incomplete beta arguments')
    if x in (0, 1):
        return float(x)
    factor = math.exp(math.lgamma(a+b) - math.lgamma(a) - math.lgamma(b)
                      + a * math.log(x) + b * math.log1p(-x))
    if x < (a + 1) / (a + b + 2):
        value = factor * _beta_fraction(a, b, x) / a
    else:
        value = 1 - factor * _beta_fraction(b, a, 1-x) / b
    if not math.isfinite(value):
        raise ValueError('undefined incomplete beta')
    return min(1.0, max(0.0, value))


def student_t_sf(t, df):
    """P(T >= t); compute the small upper tail directly, without 1-CDF."""
    if not (math.isfinite(t) and positive(df)):
        raise ValueError('undefined Student-t statistic or degrees of freedom')
    if t == 0:
        return .5
    log_ratio = math.log(abs(t)) - .5 * math.log(df)
    if log_ratio > 0:
        log_x = -2 * log_ratio - math.log1p(math.exp(-2 * log_ratio))
    else:
        # Near zero, df/(df+t²) rounds to 1; use the other beta argument.
        ratio_squared = math.exp(2 * log_ratio)
        y = ratio_squared / (1 + ratio_squared)
        center = .5 * regularized_beta(y, .5, df / 2)
        return .5 - center if t > 0 else .5 + center
    x = math.exp(log_x)
    if x == 0:
        # x can underflow before the probability does (notably the Cauchy tail).
        a = df / 2
        log_beta = math.lgamma(a) + math.lgamma(.5) - math.lgamma(a + .5)
        tail = math.exp(a * log_x - math.log(a) - log_beta - math.log(2))
    else:
        tail = .5 * regularized_beta(x, df / 2, .5)
    return tail if t > 0 else 1 - tail


def summarize(values):
    if len(values) < 2 or not all(positive(v) for v in values):
        raise ValueError('too few values or nonfinite/nonpositive run value')
    mean = statistics.mean(values)
    sd = statistics.stdev(values)  # n - 1, never population stdev.
    variance = sd * sd
    if not all(positive(v) for v in (mean, sd, variance)):
        raise ValueError('zero variance or undefined distribution')
    return {'n': len(values), 'values_ns': list(values),
            'median_ns': statistics.median(values), 'mean_ns': mean,
            'sample_stdev_ns': sd, 'cv': sd / mean}


def welch_greater(a, b):
    """One-sided independent Welch test H1: mean(a) > mean(b)."""
    sa, sb = summarize(a), summarize(b)
    va = sa['sample_stdev_ns'] ** 2 / len(a)
    vb = sb['sample_stdev_ns'] ** 2 / len(b)
    se2 = va + vb
    # Normalized weights avoid squaring large variances in Welch-Satterthwaite.
    df = 1 / ((va / se2)**2 / (len(a)-1) + (vb / se2)**2 / (len(b)-1))
    t = (sa['mean_ns'] - sb['mean_ns']) / math.sqrt(se2)
    if not (math.isfinite(t) and positive(df) and positive(se2)):
        raise ValueError('undefined Welch statistic')
    return {'t': t, 'df': df, 'p': student_t_sf(t, df)}


def inconclusive(reasons):
    return {'outcome': 'inconclusive', 'reasons': sorted(set(reasons)),
            'checks': {}, 'metrics': {}}


def evaluate_pair(physical, simulated):
    metrics = {}
    try:
        for label, groups in (('physical', physical), ('simulated', simulated)):
            if any(len(groups.get(s, [])) != N for s in SETS):
                raise ValueError(label + ': expected exactly 6 valid runs in each set')
            stats = {}
            for s in SETS:
                try:
                    stats[s] = summarize(groups[s])
                except (ValueError, OverflowError) as error:
                    raise ValueError(label + '/' + s + ': ' + str(error)) from error
            stats['delta'] = stats['V']['median_ns'] / stats['B1']['median_ns'] - 1
            stats['detection'] = welch_greater(groups['V'], groups['B1'])
            stats['aa_b1_gt_b2'] = welch_greater(groups['B1'], groups['B2'])
            stats['aa_b2_gt_b1'] = welch_greater(groups['B2'], groups['B1'])
            if not math.isfinite(stats['delta']):
                raise ValueError(label + ': undefined fractional delta')
            metrics[label] = stats
        p, s = metrics['physical'], metrics['simulated']
        if p['delta'] <= 0:
            raise ValueError('physical fractional delta <= 0')
        ratio = s['B1']['median_ns'] / p['B1']['median_ns']
        if not positive(ratio):
            raise ValueError('undefined closeness ratio')
        metrics['closeness_ratio'] = ratio
        checks = {
            'closeness': .67 <= ratio <= 1.5,
            'cv_absolute': s['B1']['cv'] <= .15,
            'cv_relative': s['B1']['cv'] <= 2 * p['B1']['cv'],
            'detection_physical': p['detection']['p'] < .001,
            'detection_simulated': s['detection']['p'] < .001,
            'delta_agreement': abs(s['delta'] - p['delta']) <= .5 * p['delta'],
        }
        for label, stats in (('physical', p), ('simulated', s)):
            for direction in ('b1_gt_b2', 'b2_gt_b1'):
                checks['aa_' + label + '_' + direction] = stats['aa_' + direction]['p'] >= .001
        failed = sorted(k for k, passed in checks.items() if not passed)
        return {'outcome': 'rejected' if failed else 'qualified',
                'reasons': failed or ['all gates passed'], 'checks': checks, 'metrics': metrics}
    except (ValueError, OverflowError, ZeroDivisionError) as error:
        return inconclusive([str(error)])


def extract_attempts(attempts):
    if not isinstance(attempts, list):
        return [], {'attempts': 0, 'invalid_attempts': 0, 'valid_runs': 0}, ['missing attempt list']
    slots = {i: [] for i in range(1, N + 1)}
    errors, values = [], []
    audit = {'attempts': len(attempts), 'invalid_attempts': 0, 'valid_runs': 0}
    for item in attempts:
        if not isinstance(item, dict):
            errors.append('attempt must be an object')
            continue
        run, attempt = item.get('run_index'), item.get('attempt')
        valid, reason = item.get('valid'), item.get('reason')
        value = item.get('value_ns')
        if type(valid) is not bool or not isinstance(reason, str) or 'value_ns' not in item:
            errors.append('attempt requires valid boolean, reason string and value_ns')
            continue
        audit['invalid_attempts'] += not valid
        if type(run) is not int or run not in slots or type(attempt) is not int or not 1 <= attempt <= 3:
            errors.append('run index or attempt number outside registered bounds')
            continue
        slots[run].append(item)
        if not valid:
            if not reason.strip() or value is not None:
                errors.append('invalid attempt needs reason and null value_ns')
        elif not positive(value):
            errors.append('nonfinite/nonpositive valid run value')
        elif 'sample_means_ns' in item:
            samples = item['sample_means_ns']
            if (not isinstance(samples, list) or len(samples) != 10
                    or not all(positive(v) for v in samples)):
                errors.append('valid stream needs ten finite positive sample means')
            elif statistics.median(samples) != value:
                errors.append('value_ns differs from median of ten sample means')
    for run, items in slots.items():
        items = sorted(items, key=lambda x: x['attempt'])
        if [item['attempt'] for item in items] != list(range(1, len(items) + 1)):
            errors.append('slot %d: attempts must be consecutive from 1' % run)
        valid_items = [item for item in items if item['valid']]
        if len(valid_items) > 1 or any(item['valid'] for item in items[:-1]):
            errors.append('slot %d: valid run replaced or retried' % run)
        if len(valid_items) == 1:
            values.append(valid_items[0]['value_ns'])
            audit['valid_runs'] += 1
    if audit['valid_runs'] != N:
        errors.append('too few valid runs: expected exactly 6 completed slots')
    return values, audit, sorted(set(errors))


def build_errors(physical, simulated):
    errors = []
    for config in (physical, simulated):
        label = config['id']
        if config.get('blocking_reason'):
            errors.append(label + ': ' + config['blocking_reason'])
        build = config.get('build', {})
        for field in ('core_revision', 'binding_revision', 'harness_revision'):
            if not re.fullmatch('[0-9a-f]{40}', str(build.get(field, ''))):
                errors.append(label + ': missing/invalid ' + field)
        if build.get('profile') != 'Release':
            errors.append(label + ': build must be Release')
        if build.get('slowdown_flag') != 'repeat-timed-operation-twice':
            errors.append(label + ': missing registered slowdown flag')
        hashes = build.get('artifact_sha256', {})
        if not isinstance(hashes, dict) or any(
                not re.fullmatch('[0-9a-f]{64}', str(hashes.get(s, ''))) for s in SETS):
            errors.append(label + ': missing/invalid executed artifact hash')
        runtime = config.get('runtime')
        if (not isinstance(runtime, dict) or not runtime
                or any(not isinstance(v, str) or not v.strip() or 'TODO' in v
                       for v in runtime.values())):
            errors.append(label + ': missing/unpinned runtime')
    for field in ('core_revision', 'binding_revision', 'harness_revision', 'profile', 'slowdown_flag'):
        if physical.get('build', {}).get(field) != simulated.get('build', {}).get(field):
            errors.append('unmatched build: ' + field)
    return errors


def decide(document):
    if (not isinstance(document, dict) or type(document.get('schema_version')) is not int
            or document.get('schema_version') != 1
            or type(document.get('synthetic')) is not bool):
        raise ValueError('expected schema_version 1 and explicit synthetic boolean')
    configs = document.get('configurations')
    if not isinstance(configs, list) or not configs:
        raise ValueError('configurations must be a nonempty list')
    by_id = {}
    for config in configs:
        if not isinstance(config, dict):
            raise ValueError('configuration must be an object')
        ident = config.get('id')
        if not isinstance(ident, str) or not re.fullmatch('[a-z0-9][a-z0-9-]*', ident) or ident in by_id:
            raise ValueError('invalid or duplicate configuration id')
        if config.get('kind') not in ('physical', 'simulated') or config.get('platform') not in ('ios', 'android'):
            raise ValueError('invalid configuration kind/platform')
        if not isinstance(config.get('cases'), dict) or set(config['cases']) - CATEGORIES.keys():
            raise ValueError('invalid or unknown case IDs')
        if not isinstance(config.get('build', {}), dict):
            raise ValueError('build must be an object')
        if config.get('blocking_reason') is not None and not isinstance(config['blocking_reason'], str):
            raise ValueError('blocking_reason must be a string or null')
        by_id[ident] = config
    comparisons, categories = [], {}
    for config in sorted(configs, key=lambda x: x['id']):
        if config['kind'] == 'physical':
            if config.get('reference') is not None:
                raise ValueError('physical configuration cannot reference another configuration')
            continue
        reference = config.get('reference')
        if not isinstance(reference, str) or reference not in by_id:
            raise ValueError('candidate must reference an existing physical configuration')
        physical = by_id[reference]
        if physical['kind'] != 'physical' or physical['platform'] != config['platform']:
            raise ValueError('reference must be physical on the same platform')
        common_errors = build_errors(physical, config)
        counts = {category: dict.fromkeys(('qualified', 'rejected', 'inconclusive'), 0)
                  for category in sorted(set(CATEGORIES.values()))}
        for case, category in sorted(CATEGORIES.items()):
            errors, data, audit = common_errors[:], {}, {}
            for label, source in (('physical', physical), ('simulated', config)):
                entry = source['cases'].get(case, {})
                if not isinstance(entry, dict) or not isinstance(entry.get('sets', {}), dict):
                    raise ValueError('case and sets must be objects')
                if set(entry.get('sets', {})) - set(SETS):
                    raise ValueError('unknown set name')
                if entry.get('blocking_reason'):
                    errors.append(label + ': ' + str(entry['blocking_reason']))
                data[label], audit[label] = {}, {}
                for s in SETS:
                    values, accounting, problems = extract_attempts(entry.get('sets', {}).get(s))
                    data[label][s], audit[label][s] = values, accounting
                    errors.extend(label + '/' + s + ': ' + problem for problem in problems)
            result = inconclusive(errors) if errors else evaluate_pair(data['physical'], data['simulated'])
            result.update(case=case, category=category, platform=config['platform'],
                          candidate=config['id'], physical=reference, audit=audit)
            comparisons.append(result)
            counts[category][result['outcome']] += 1
        categories[config['id']] = counts
    if not comparisons:
        raise ValueError('at least one candidate comparison is required')
    return {'schema_version': 1, 'protocol': '858-mobile-ac12-n6-v1',
            'synthetic': document['synthetic'], 'categories': categories,
            'configurations': [{k: v for k, v in c.items() if k != 'cases'}
                               for c in sorted(configs, key=lambda x: x['id'])],
            'comparisons': comparisons}


def render_json(value):
    """Sorted UTF-8 JSON, 12 digits after decimal in scientific float notation."""
    def emit(item, level=0):
        if isinstance(item, float):
            if not math.isfinite(item):
                raise ValueError('nonfinite report number')
            return format(0.0 if item == 0 else item, '.12e')
        if isinstance(item, dict):
            pairs = [json.dumps(k, ensure_ascii=True) + ': ' + emit(v, level+1)
                     for k, v in sorted(item.items())]
            left, right = '{', '}'
        elif isinstance(item, list):
            pairs = [emit(v, level+1) for v in item]
            left, right = '[', ']'
        else:
            return json.dumps(item, ensure_ascii=True, allow_nan=False)
        if not pairs:
            return left + right
        return left + '\n' + ',\n'.join('  ' * (level+1) + p for p in pairs) + '\n' + '  ' * level + right
    return emit(value) + '\n'


def render_markdown(report):
    def number(v):
        return format(v, '.9e')

    def cell(v):
        return str(v).replace('|', '\\|').replace('\r', ' ').replace('\n', ' ')

    lines = ['# Mobile qualification decisions', '',
             '**SYNTHETIC — NOT REAL DATA; tests only.**' if report['synthetic'] else
             'Computed decisions; provenance and study completion still require evidence review.', '',
             'Medians are ns; CV and delta are fractions. A/A lists p(B1>B2), p(B2>B1).', '',
             '| Candidate / case | Category | B1 median phys / sim | Ratio | CV phys / sim | Delta phys / sim | p(V>B1) phys / sim | A/A physical | A/A simulated | Outcome / reason |',
             '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |']
    for row in report['comparisons']:
        m = row['metrics']
        fields = ['—'] * 7
        if m:
            p, s = m['physical'], m['simulated']
            fields = [number(p['B1']['median_ns']) + ' / ' + number(s['B1']['median_ns']),
                      number(m['closeness_ratio']),
                      number(p['B1']['cv']) + ' / ' + number(s['B1']['cv']),
                      number(p['delta']) + ' / ' + number(s['delta']),
                      number(p['detection']['p']) + ' / ' + number(s['detection']['p']),
                      number(p['aa_b1_gt_b2']['p']) + ', ' + number(p['aa_b2_gt_b1']['p']),
                      number(s['aa_b1_gt_b2']['p']) + ', ' + number(s['aa_b2_gt_b1']['p'])]
        cells = [row['candidate'] + ' / ' + row['case'], row['category']] + fields + [
            row['outcome'] + ': ' + '; '.join(row['reasons'])]
        lines.append('| ' + ' | '.join(cell(v) for v in cells) + ' |')
    lines += ['', 'Category counts only; they never override a case.', '',
              '| Candidate | Category | Qualified | Rejected | Inconclusive |',
              '| --- | --- | --- | --- | --- |']
    for candidate, categories in sorted(report['categories'].items()):
        for category, counts in sorted(categories.items()):
            lines.append('| %s | %s | %d | %d | %d |' % (
                candidate, category, counts['qualified'], counts['rejected'], counts['inconclusive']))
    return '\n'.join(lines) + '\n'


def load_raw(path):
    path = Path(path)
    if path.is_dir():
        path = path / 'raw-samples.json'

    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate JSON key: ' + key)
            result[key] = value
        return result
    # Tolerate NaN/Infinity at input so bad run values yield inconclusive, never pass.
    return json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=unique_pairs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', help='JSON file or directory containing raw-samples.json')
    parser.add_argument('--output', required=True, help='report prefix (writes .json and .md)')
    args = parser.parse_args()
    try:
        report = decide(load_raw(args.raw))
        prefix = Path(args.output)
        prefix.with_suffix('.json').write_text(render_json(report), encoding='utf-8', newline='\n')
        prefix.with_suffix('.md').write_text(render_markdown(report), encoding='utf-8', newline='\n')
    except (ValueError, OSError, OverflowError) as error:
        print('error: ' + str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
