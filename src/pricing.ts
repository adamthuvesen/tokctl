import type { UsageEvent } from './types.js';

interface PriceEntry {
  input: number;
  output: number;
  cacheRead?: number;
  cacheWrite?: number;
}

// USD per 1M tokens. Last verified 2026-04.
const PRICES: Record<string, PriceEntry> = {
  // Anthropic
  'claude-opus-4-7':   { input: 15,   output: 75,   cacheRead: 1.5,   cacheWrite: 18.75 },
  'claude-opus-4-6':   { input: 15,   output: 75,   cacheRead: 1.5,   cacheWrite: 18.75 },
  'claude-sonnet-4-6': { input: 3,    output: 15,   cacheRead: 0.3,   cacheWrite: 3.75 },
  'claude-sonnet-4-5': { input: 3,    output: 15,   cacheRead: 0.3,   cacheWrite: 3.75 },
  'claude-haiku-4-5':  { input: 1,    output: 5,    cacheRead: 0.1,   cacheWrite: 1.25 },

  // OpenAI (Codex). Sources: OpenAI pricing page (current), OpenRouter legacy cards.
  'gpt-5':                { input: 1.25, output: 10,   cacheRead: 0.125 },
  'gpt-5-codex':          { input: 1.25, output: 10,   cacheRead: 0.125 },
  'gpt-5-codex-mini':     { input: 0.25, output: 2,    cacheRead: 0.025 },
  'gpt-5.1':              { input: 1.75, output: 14,   cacheRead: 0.175 },
  'gpt-5.1-codex-max':    { input: 1.75, output: 14,   cacheRead: 0.175 },
  'gpt-5.1-codex-mini':   { input: 0.25, output: 2,    cacheRead: 0.025 },
  'gpt-5.2':              { input: 1.75, output: 14,   cacheRead: 0.175 },
  'gpt-5.2-codex':        { input: 1.75, output: 14,   cacheRead: 0.175 },
  'gpt-5.3':              { input: 1.75, output: 14,   cacheRead: 0.175 },
  'gpt-5.3-codex':        { input: 1.75, output: 14,   cacheRead: 0.175 },
  'gpt-5.4':              { input: 2.5,  output: 15,   cacheRead: 0.25 },
  'gpt-5.4-codex':        { input: 2.5,  output: 15,   cacheRead: 0.25 },
  'gpt-5.4-mini':         { input: 0.75, output: 4.5,  cacheRead: 0.075 },
  'gpt-5.4-nano':         { input: 0.2,  output: 1.25, cacheRead: 0.02 },
  'o4-mini':              { input: 1.1,  output: 4.4,  cacheRead: 0.275 },
};

export function normalizeModelId(model: string): string {
  // Claude IDs sometimes ship with a -YYYYMMDD suffix. Strip it for lookup.
  return model.replace(/-\d{8}$/, '');
}

export function costOf(event: UsageEvent, unknownModels?: Set<string>): number {
  const key = normalizeModelId(event.model);
  const p = PRICES[key];
  if (!p) {
    if (unknownModels) unknownModels.add(event.model);
    return 0;
  }
  const M = 1_000_000;
  return (
    (event.inputTokens * p.input) / M +
    (event.outputTokens * p.output) / M +
    (event.cacheReadTokens * (p.cacheRead ?? 0)) / M +
    (event.cacheWriteTokens * (p.cacheWrite ?? 0)) / M
  );
}

export function hasPrice(model: string): boolean {
  return PRICES[normalizeModelId(model)] !== undefined;
}
