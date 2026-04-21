import { describe, it, expect } from 'vitest';
import os from 'node:os';
import path from 'node:path';
import { resolveRoots } from '../src/paths.js';

describe('resolveRoots', () => {
  it('prefers CLI flag over env and defaults', () => {
    const { roots, userSupplied } = resolveRoots({
      flag: '/a/claude,/b/claude',
      tokctlEnv: '/nope',
      toolEnv: '/also-nope',
      defaults: ['/d'],
    });
    expect(userSupplied).toBe(true);
    expect(roots).toEqual(['/a/claude', '/b/claude']);
  });

  it('falls back to TOKCTL_* env when no flag', () => {
    const { roots, userSupplied } = resolveRoots({
      tokctlEnv: '/env/a,/env/b',
      toolEnv: '/should-not-win',
      defaults: ['/d'],
    });
    expect(userSupplied).toBe(true);
    expect(roots).toEqual(['/env/a', '/env/b']);
  });

  it('joins a suffix when falling back to tool-native env', () => {
    const { roots, userSupplied } = resolveRoots({
      toolEnv: '/alt/codex',
      toolEnvSuffix: 'sessions',
      defaults: ['/d'],
    });
    expect(userSupplied).toBe(true);
    expect(roots).toEqual([path.join('/alt/codex', 'sessions')]);
  });

  it('uses defaults when nothing is set', () => {
    const { roots, userSupplied } = resolveRoots({ defaults: ['~/x', '/y'] });
    expect(userSupplied).toBe(false);
    expect(roots).toEqual([path.join(os.homedir(), 'x'), '/y']);
  });

  it('ignores empty-string flags', () => {
    const { roots, userSupplied } = resolveRoots({ flag: '   ', defaults: ['/d'] });
    expect(userSupplied).toBe(false);
    expect(roots).toEqual(['/d']);
  });
});
