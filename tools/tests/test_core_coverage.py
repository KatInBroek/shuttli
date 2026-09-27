"""The release gate must not accept rounded, missing, or empty evidence."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('coverage_gate', Path(__file__).parents[1] / 'check_core_coverage.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class CoverageGateTests(unittest.TestCase):
    def report(self, covered=96, count=100):
        return {'data': [{'files': [{'filename': '/build/crates/core/src/lib.rs',
                                    'summary': {m: {'count': count, 'covered': covered}
                                                for m in gate.METRICS}}]}]}

    def test_only_verified_declarations_can_lack_llvm_regions(self):
        self.assertTrue(gate.declarations_only('//! crate docs\n#![no_std]\npub mod sync;\npub use sync::Publication as LocalPublication;'))
        for source in ('pub fn hidden() {}', 'include!("hidden.rs");', 'const X: u8 = run();',
                       '#[cfg(test)]\npub mod sync;', 'pub mod sync; pub fn hidden() {}'):
            self.assertFalse(gate.declarations_only(source))

    def test_strict_boundary(self):
        for covered in (0, 94, 95):
            with self.subTest(covered=covered), self.assertRaises(ValueError):
                gate.evaluate(self.report(covered), ['crates/core/src/lib.rs'])
        self.assertEqual(gate.evaluate(self.report(), ['crates/core/src/lib.rs'])['metrics']['functions']['percent'], 96)

    def test_missing_or_extra_file_cannot_shrink_denominator(self):
        with self.assertRaises(ValueError):
            gate.evaluate(self.report(), ['crates/core/src/lib.rs', 'crates/core/src/sync.rs'])
        with self.assertRaises(ValueError):
            gate.evaluate(self.report(), [])

    def test_test_code_or_duplicate_entries_cannot_inflate_coverage(self):
        report = self.report()
        report['data'][0]['files'].append(copy.deepcopy(report['data'][0]['files'][0]))
        with self.assertRaises(ValueError):
            gate.evaluate(report, ['crates/core/src/lib.rs'])
        report['data'][0]['files'][-1]['filename'] = '/build/crates/core/src/tests/sync.rs'
        with self.assertRaises(ValueError):
            gate.evaluate(report, ['crates/core/src/lib.rs'])

    def test_every_metric_must_pass_and_counts_must_be_valid(self):
        for metric in gate.METRICS:
            for count, covered in ((100, 95), (0, 0), (100, 101), (100, -1)):
                report = self.report()
                report['data'][0]['files'][0]['summary'][metric] = {'count': count, 'covered': covered}
                with self.subTest(metric=metric, count=count, covered=covered), self.assertRaises(ValueError):
                    gate.evaluate(report, ['crates/core/src/lib.rs'])


if __name__ == '__main__':
    unittest.main()
