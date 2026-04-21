import os from 'node:os';
import path from 'node:path';

export function resolveCachePath(env: NodeJS.ProcessEnv = process.env): string {
  const override = env.AIUSAGE_CACHE_DIR?.trim();
  if (override) return path.join(override, 'aiusage.db');
  const xdg = env.XDG_CACHE_HOME?.trim();
  const base = xdg ? xdg : path.join(os.homedir(), '.cache');
  return path.join(base, 'aiusage', 'aiusage.db');
}
