import os from 'node:os';
import path from 'node:path';

export interface ResolveRootsInput {
  flag?: string;
  aiusageEnv?: string;
  toolEnv?: string;
  toolEnvSuffix?: string;
  defaults: string[];
}

export interface ResolveRootsResult {
  roots: string[];
  userSupplied: boolean;
}

function expand(p: string): string {
  const trimmed = p.trim();
  if (!trimmed) return trimmed;
  if (trimmed === '~') return os.homedir();
  if (trimmed.startsWith('~/')) return path.join(os.homedir(), trimmed.slice(2));
  return trimmed;
}

function splitCsv(value: string): string[] {
  return value
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .map(expand);
}

export function resolveRoots(input: ResolveRootsInput): ResolveRootsResult {
  if (input.flag && input.flag.trim()) {
    return { roots: splitCsv(input.flag), userSupplied: true };
  }
  if (input.aiusageEnv && input.aiusageEnv.trim()) {
    return { roots: splitCsv(input.aiusageEnv), userSupplied: true };
  }
  if (input.toolEnv && input.toolEnv.trim()) {
    const parts = splitCsv(input.toolEnv);
    const suffix = input.toolEnvSuffix;
    const roots = suffix ? parts.map((p) => path.join(p, suffix)) : parts;
    return { roots, userSupplied: true };
  }
  return { roots: input.defaults.map(expand), userSupplied: false };
}
