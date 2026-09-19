#!/usr/bin/env python3
"""Guard the reviewed schema boundary of the identity audit (EVD-0023).

Hashes detect drift, not semantics. Changing a hash requires reviewing the
changed declaration and its identity/conversion classification in the ledger.
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / 'plans/v2/evidence/phase-00b/EVD-0023-schema-review.json'
LEDGER = ROOT / 'plans/v2/inventories/identities.md'
SCHEMAS = ('project', 'bundle-metadata', 'patch')


def fingerprint(value: dict) -> str:
    """Hash a complete declaration, including nested maps and variants."""
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def declarations(schemas: dict) -> dict:
    result = {}
    for name in SCHEMAS:
        schema = schemas[name]
        result[f'{name}:root'] = {k: v for k, v in schema.items() if k != '$defs'}
        for key, value in schema.get('$defs', {}).items():
            result[f'{name}:{key}'] = value
    return result


LEDGER_HEADER = (
    'ID', 'Concept/reference', 'Current type/encoding', 'Producers', 'Consumers',
    'Persistence', 'Known problem', 'Proposed V2 newtype/rule', 'Migration', 'ADR', 'Status',
)
CONVERSION_HEADER = (
    'Entry', 'Disposition, source key and reference handling', 'First consumer / required verification',
)
STATUSES = {'Discovered', 'Investigating', 'Classified', 'Verified', 'Needs review'}


def table_rows(text: str, header: tuple) -> list[list[str]]:
    """Read IDN rows only in contiguous GFM tables with this header and separator."""
    rows = []
    in_table = False
    pending_header = False
    for line in text.splitlines():
        if not line.startswith('|'):
            in_table = pending_header = False
            continue
        cells = [c.strip() for c in re.split(r'(?<!\\)\|', line)[1:-1]]
        if tuple(cells) == header:
            pending_header = True
            in_table = False
            continue
        if pending_header:
            in_table = len(cells) == len(header) and all(
                re.fullmatch(r':?-+:?', cell) for cell in cells)
            pending_header = False
            continue
        if in_table and re.fullmatch(r'IDN-\d{4}', cells[0] if cells else ''):
            if len(cells) != len(header):
                raise ValueError(f'{cells[0]}: malformed {header[0]} table row')
            rows.append(cells)
    return rows


def ledger_entries(text: str) -> dict:
    """Only contiguous concept tables define IDs, never mapping tables or prose."""
    entries = {}
    for cells in table_rows(text, LEDGER_HEADER):
        key = cells[0]
        if key in entries:
            raise ValueError(f'duplicate ledger entry: {key}')
        entries[key] = cells
    return entries


def check(schemas: dict, manifest: dict, ledger: str) -> list[str]:
    errors = []
    if set(manifest) != {'declarations', 'outside_schema'}:
        return ['invalid manifest fields']
    try:
        entries = ledger_entries(ledger)
    except ValueError as error:
        return [str(error)]
    if not entries:
        errors.append('no ledger entries')
    for key, cells in entries.items():
        if cells[-1] in {'Classified', 'Verified'} and any(not cell for cell in cells):
            errors.append(f'{key}: blank audit field')
        if cells[-1] not in STATUSES:
            errors.append(f'{key}: unknown entry status')
    actual = declarations(schemas)
    reviewed = manifest['declarations']
    for key in sorted(actual.keys() - reviewed.keys()):
        errors.append(f'unreviewed declaration: {key}')
    for key in sorted(reviewed.keys() - actual.keys()):
        errors.append(f'stale declaration: {key}')
    covered = set()
    for key, row in reviewed.items():
        if set(row) != {'sha256', 'entries', 'reason'} or not row['reason'].strip():
            errors.append(f'{key}: incomplete classification')
            continue
        if key in actual and row['sha256'] != fingerprint(actual[key]):
            errors.append(f'changed declaration: {key}')
        if len(row['entries']) != len(set(row['entries'])):
            errors.append(f'{key}: duplicate entry claim')
        covered.update(row['entries'])
    for key, reason in manifest['outside_schema'].items():
        if not reason.strip():
            errors.append(f'{key}: missing outside-schema reason')
        covered.add(key)
    for key in sorted(covered - entries.keys()):
        errors.append(f'undefined ledger entry: {key}')
    for key in sorted(entries.keys() - covered):
        errors.append(f'uncovered ledger entry: {key}')
    # Completed classifications need one disposition; reopening may leave work pending.
    try:
        conversion_rows = table_rows(ledger, CONVERSION_HEADER)
        dispositions = [row[0] for row in conversion_rows]
    except ValueError as error:
        return errors + [str(error)]
    for row in conversion_rows:
        if row[0] in entries and entries[row[0]][-1] in {'Classified', 'Verified'}:
            if any(not cell for cell in row):
                errors.append(f'{row[0]}: blank conversion disposition field')
    for key, cells in entries.items():
        count = dispositions.count(key)
        if count > 1 or (cells[-1] in {'Classified', 'Verified'} and count != 1):
            errors.append(f'{key}: expected one conversion disposition')
    for key in sorted(set(dispositions) - entries.keys()):
        errors.append(f'undefined conversion entry: {key}')
    # Patch roots differ only in document-level schema metadata.
    patch = schemas['patch']
    patch_root = {k: v for k, v in patch.items() if k not in ('$schema', '$defs', 'title')}
    if patch_root != schemas['project']['$defs']['Patch']:
        errors.append('standalone patch differs from reviewed project Patch')
    for key, value in patch.get('$defs', {}).items():
        if schemas['project']['$defs'].get(key) != value:
            errors.append(f'standalone patch definition differs: {key}')
    return errors


def load_inputs() -> tuple:
    schemas = {name: json.loads((ROOT / f'schemas/{name}.schema.json').read_text())
               for name in SCHEMAS}
    return schemas, json.loads(MANIFEST.read_text()), LEDGER.read_text()


def main() -> int:
    try:
        inputs = load_inputs()
        errors = check(*inputs)
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        print(f'identity coverage input error: {error}', file=sys.stderr)
        return 1
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    entries = ledger_entries(inputs[2])
    statuses = ', '.join(f'{status}={sum(row[-1] == status for row in entries.values())}'
                         for status in sorted({row[-1] for row in entries.values()}))
    print(f'Identity audit: {len(declarations(inputs[0]))} schema declarations, '
          f'{len(entries)} entries ({statuses}); patch schema agrees.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
