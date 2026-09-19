"""EVD-0023 controls: mutate real reviewed inputs before the baseline check."""
import copy
import re
import unittest
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_identity_coverage as audit  # noqa: E402


class IdentityCoverageTests(unittest.TestCase):
    def setUp(self):
        self.schemas, self.manifest, self.ledger = copy.deepcopy(audit.load_inputs())

    def errors(self):
        return audit.check(self.schemas, self.manifest, self.ledger)

    def test_added_field_is_not_hidden_by_unchanged_definition_count(self):
        self.schemas['project']['$defs']['Note']['properties']['source_note_id'] = {'type': 'integer'}
        self.assertIn('changed declaration: project:Note', self.errors())

    def test_added_definition(self):
        self.schemas['project']['$defs']['NewReference'] = {'type': 'integer'}
        self.assertIn('unreviewed declaration: project:NewReference', self.errors())

    def test_removed_definition(self):
        del self.schemas['project']['$defs']['NoteId']
        self.assertIn('stale declaration: project:NoteId', self.errors())

    def test_graph_map_keys(self):
        node = self.schemas['project']['$defs']['ModGraph']['properties']['nodes']
        node['patternProperties']['other'] = {'type': 'string'}
        self.assertIn('changed declaration: project:ModGraph', self.errors())

    def test_tuple_endpoint(self):
        node = self.schemas['project']['$defs']['ConnectionState']['properties']['from']
        node['maxItems'] = 3
        self.assertIn('changed declaration: project:ConnectionState', self.errors())

    def test_missing_patch_definition(self):
        key = next(iter(self.schemas['patch']['$defs']))
        del self.schemas['patch']['$defs'][key]
        self.assertIn(f'stale declaration: patch:{key}', self.errors())

    def test_patch_semantic_divergence_even_after_hash_refresh(self):
        self.schemas['patch']['properties']['new_reference'] = {'type': 'integer'}
        self.manifest['declarations']['patch:root']['sha256'] = audit.fingerprint(
            audit.declarations(self.schemas)['patch:root'])
        self.assertIn('standalone patch differs from reviewed project Patch', self.errors())

    def test_uncovered_runtime_entry(self):
        del self.manifest['outside_schema']['IDN-0032']
        self.assertIn('uncovered ledger entry: IDN-0032', self.errors())

    def test_undefined_entry_is_not_defined_by_conversion_table(self):
        self.ledger = '\n'.join(line for line in self.ledger.splitlines()
                                if not line.startswith('| IDN-0032 | Hub'))
        self.assertIn('undefined ledger entry: IDN-0032', self.errors())

    def test_blank_migration_cell(self):
        rows = self.ledger.splitlines()
        for i, row in enumerate(rows):
            if row.startswith('| IDN-0032 | Hub'):
                cells = re.split(r'(?<!\\)\|', row)
                cells[9] = ' '
                rows[i] = '|'.join(cells)
        self.ledger = '\n'.join(rows)
        self.assertIn('IDN-0032: blank audit field', self.errors())

    def test_duplicate_ledger_entry(self):
        row = next(line for line in self.ledger.splitlines() if line.startswith('| IDN-0032 | Hub'))
        self.ledger = self.ledger.replace(row, row + '\n' + row)
        self.assertIn('duplicate ledger entry: IDN-0032', self.errors())

    def test_missing_conversion_disposition(self):
        self.ledger = '\n'.join(line for line in self.ledger.splitlines()
                                if not line.startswith('| IDN-0032 | Defer'))
        self.assertIn('IDN-0032: expected one conversion disposition', self.errors())

    def test_duplicate_conversion_disposition(self):
        row = next(line for line in self.ledger.splitlines() if line.startswith('| IDN-0032 | Defer'))
        self.ledger = self.ledger.replace(row, row + '\n' + row)
        self.assertIn('IDN-0032: expected one conversion disposition', self.errors())

    def test_missing_reason(self):
        self.manifest['declarations']['project:Note']['reason'] = ''
        self.assertIn('project:Note: incomplete classification', self.errors())

    def test_blank_line_ends_concept_table(self):
        self.ledger = self.ledger.replace('| IDN-0041 | Session', '\n| IDN-0041 | Session')
        self.assertIn('undefined ledger entry: IDN-0041', self.errors())

    def test_unrelated_eleven_column_table_does_not_define_entry(self):
        row = next(line for line in self.ledger.splitlines() if line.startswith('| IDN-0032 | Hub'))
        self.ledger = self.ledger.replace(row, '')
        self.ledger += '\n| ID | A | B | C | D | E | F | G | H | I | J |\n'
        self.ledger += '|---|---|---|---|---|---|---|---|---|---|---|\n' + row
        self.assertIn('undefined ledger entry: IDN-0032', self.errors())

    def test_header_without_separator_is_not_table(self):
        header = '| ID | Concept/reference | Current type/encoding | Producers | Consumers | Persistence | Known problem | Proposed V2 newtype/rule | Migration | ADR | Status |'
        separator = '|---|---|---|---|---|---|---|---|---|---|---|'
        self.ledger = self.ledger.replace(header + '\n' + separator, header)
        self.assertIn('undefined ledger entry: IDN-0032', self.errors())

    def test_conversion_row_outside_its_table_does_not_count(self):
        row = next(line for line in self.ledger.splitlines() if line.startswith('| IDN-0032 | Defer'))
        self.ledger = self.ledger.replace(row, '')
        self.ledger += '\n' + row
        self.assertIn('IDN-0032: expected one conversion disposition', self.errors())

    def test_verified_is_valid(self):
        self.ledger = self.ledger.replace(' | Classified |', ' | Verified |')
        self.assertEqual([], self.errors())

    def test_reopened_entry_can_record_incomplete_migration(self):
        rows = self.ledger.splitlines()
        for i, row in enumerate(rows):
            if row.startswith('| IDN-0032 | Hub'):
                cells = re.split(r'(?<!\\)\|', row)
                cells[9] = ' '
                cells[-2] = ' Needs review '
                rows[i] = '|'.join(cells)
        self.ledger = '\n'.join(rows)
        self.assertEqual([], self.errors())

    def test_unknown_status_is_rejected(self):
        self.ledger = self.ledger.replace(' | Classified |', ' | Accepted |')
        self.assertIn('IDN-0032: unknown entry status', self.errors())

    def test_blank_conversion_rule_is_rejected(self):
        row = next(line for line in self.ledger.splitlines() if line.startswith('| IDN-0032 | Defer'))
        cells = row.split('|')
        cells[2] = ' '
        self.ledger = self.ledger.replace(row, '|'.join(cells))
        self.assertIn('IDN-0032: blank conversion disposition field', self.errors())

    def test_zz_repository_baseline(self):
        self.assertEqual([], self.errors())


if __name__ == '__main__':
    unittest.main()
