import os from 'node:os';
import path from 'node:path';

export function resolveCachePath(env: NodeJS.ProcessEnv = process.env): string {
  const override = env.TOKCTL_CACHE_DIR?.trim();
  if (override) return path.join(override, 'tokctl.db');
  const xdg = env.XDG_CACHE_HOME?.trim();
  const base = xdg ? xdg : path.join(os.homedir(), '.cache');
  return path.join(base, 'tokctl', 'tokctl.db');
}
