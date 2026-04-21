export function parseSince(value: string | undefined): Date | undefined {
  if (!value) return undefined;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    throw new Error(`--since must be YYYY-MM-DD, got "${value}"`);
  }
  const d = new Date(`${value}T00:00:00`);
  if (Number.isNaN(d.getTime())) {
    throw new Error(`--since not a valid date: "${value}"`);
  }
  return d;
}

export function parseUntil(value: string | undefined): Date | undefined {
  if (!value) return undefined;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    throw new Error(`--until must be YYYY-MM-DD, got "${value}"`);
  }
  const d = new Date(`${value}T23:59:59.999`);
  if (Number.isNaN(d.getTime())) {
    throw new Error(`--until not a valid date: "${value}"`);
  }
  return d;
}
