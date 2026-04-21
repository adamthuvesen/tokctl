import { describe, it, expect } from 'vitest';
import { planIngest, type Discovery } from '../src/ingest/plan.js';
import type { FileManifestRow } from '../src/store/writes.js';

function mf(ov: Partial<FileManifestRow> = {}): FileManifestRow {
  return {
    path: '/x.jsonl',
    source: 'claude',
    project: null,
    size: 1000,
    mtime_ns: 1_000n,
    last_offset: 1000,
    n_events: 5,
    session_id: null,
    model: null,
    ...ov,
  };
}

describe('planIngest', () => {
  const now = Date.now();
  const tenSecondsAgoNs = BigInt(now - 10_000) * 1_000_000n;
  const oneDayAgoNs = BigInt(now - 24 * 60 * 60 * 1000) * 1_000_000n;

  it('unchanged file → skip', () => {
    const manifest = new Map([['/a.jsonl', mf({ path: '/a.jsonl', mtime_ns: oneDayAgoNs })]]);
    const discovery: Discovery = {
      files: [{ path: '/a.jsonl', source: 'claude', project: null, size: 1000, mtimeNs: oneDayAgoNs }],
      unchangedPaths: new Set(),
    };
    const plan = planIngest({ manifest, discovery, now });
    expect(plan.toSkip).toEqual(['/a.jsonl']);
    expect(plan.toTail).toHaveLength(0);
    expect(plan.toFullParse).toHaveLength(0);
  });

  it('grown file → tail-read', () => {
    const manifest = new Map([['/a.jsonl', mf({ path: '/a.jsonl', last_offset: 500, size: 500, mtime_ns: oneDayAgoNs })]]);
    const discovery: Discovery = {
      files: [{ path: '/a.jsonl', source: 'claude', project: null, size: 800, mtimeNs: tenSecondsAgoNs }],
      unchangedPaths: new Set(),
    };
    const plan = planIngest({ manifest, discovery, now });
    expect(plan.toTail).toEqual([{ file: discovery.files[0], fromOffset: 500 }]);
  });

  it('truncated file → full re-parse', () => {
    const manifest = new Map([['/a.jsonl', mf({ path: '/a.jsonl', last_offset: 1000, size: 1000 })]]);
    const discovery: Discovery = {
      files: [{ path: '/a.jsonl', source: 'claude', project: null, size: 300, mtimeNs: 1000n }],
      unchangedPaths: new Set(),
    };
    const plan = planIngest({ manifest, discovery, now });
    expect(plan.toFullParse).toHaveLength(1);
  });

  it('new file → full parse', () => {
    const manifest = new Map<string, FileManifestRow>();
    const discovery: Discovery = {
      files: [{ path: '/new.jsonl', source: 'claude', project: null, size: 300, mtimeNs: 1000n }],
      unchangedPaths: new Set(),
    };
    const plan = planIngest({ manifest, discovery, now });
    expect(plan.toFullParse).toHaveLength(1);
    expect(plan.toSkip).toHaveLength(0);
  });

  it('deleted file → purge', () => {
    const manifest = new Map([['/gone.jsonl', mf({ path: '/gone.jsonl' })]]);
    const discovery: Discovery = { files: [], unchangedPaths: new Set() };
    const plan = planIngest({ manifest, discovery, now });
    expect(plan.toPurge).toEqual(['/gone.jsonl']);
  });

  it('unchanged dir: paths in unchangedPaths go to toSkip not toPurge', () => {
    const manifest = new Map([['/a.jsonl', mf({ path: '/a.jsonl', mtime_ns: oneDayAgoNs })]]);
    const discovery: Discovery = { files: [], unchangedPaths: new Set(['/a.jsonl']) };
    const plan = planIngest({ manifest, discovery, now });
    expect(plan.toSkip).toEqual(['/a.jsonl']);
    expect(plan.toPurge).toHaveLength(0);
  });
});
