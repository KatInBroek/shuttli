#!/usr/bin/env python3
"""Fail closed unless every production core source is measured above 95%.

Input: cargo llvm-cov -p shuttli-core --locked --json --output-path <path>.
Only test harness files under crates/core/src/tests are excluded. No production
functions or source lines may be selectively excluded to satisfy this gate.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
METRICS = ('functions', 'lines', 'regions')


def evaluate(report, expected):
    files = [f for group in report['data'] for f in group['files']]
    measured = {}
    for file in files:
        path = file['filename'].replace('\\', '/')
        marker = 'crates/core/src/'
        if marker not in path:
            raise ValueError('unexpected file outside the core-only report: ' + path)
        relative = marker + path.split(marker, 1)[1]
        if relative in measured:
            raise ValueError('duplicate coverage entry: ' + relative)
        measured[relative] = file['summary']
    if set(measured) != set(expected):
        raise ValueError('coverage scope differs from production core sources: ' +
                         str(sorted(set(measured) ^ set(expected))))
    totals = {m: {'count': 0, 'covered': 0} for m in METRICS}
    for summary in measured.values():
        for metric in METRICS:
            count, covered = summary[metric]['count'], summary[metric]['covered']
            if not isinstance(count, int) or not isinstance(covered, int) or not 0 <= covered <= count:
                raise ValueError('invalid coverage counts')
            totals[metric]['count'] += count
            totals[metric]['covered'] += covered
    for metric, values in totals.items():
        count, covered = values['count'], values['covered']
        # Compare exact integer counts. Rounded 95.00% must not pass >95%.
        if count == 0 or covered * 100 <= count * 95:
            raise ValueError(f'{metric} coverage must be strictly >95%: {covered}/{count}')
        values['percent'] = round(100 * covered / count, 4)
    return {'scope': sorted(measured), 'threshold': '>95%', 'metrics': totals,
            'branches': 'not measured by this stable-toolchain run'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report')
    parser.add_argument('--summary-output')
    args = parser.parse_args()
    production = sorted(p for p in (ROOT / 'crates/core/src').rglob('*.rs')
                        if 'tests' not in p.relative_to(ROOT / 'crates/core/src').parts)
    names = [p.relative_to(ROOT).as_posix() for p in production]
    summary = evaluate(json.loads(Path(args.report).read_text()), names)
    summary['source_sha256'] = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                                for name in names}
    if args.summary_output:
        output = Path(args.summary_output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(summary, indent=2) + '\n')
    lines = ['## Core coverage (>95% gate)', '']
    for metric, value in summary['metrics'].items():
        lines.append(f"- {metric}: {value['covered']}/{value['count']} ({value['percent']}%)")
    lines.append('- Branch coverage: not measured; test code excluded.')
    text = '\n'.join(lines) + '\n'
    print(text)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as f:
            f.write(text)


if __name__ == '__main__':
    main()
