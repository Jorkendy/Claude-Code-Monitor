export type Tokens = {
  input: number;
  output: number;
  cache_creation: number;
  cache_read: number;
};

export function fmtUSD(v: number | null | undefined): string {
  if (v == null) return "—";
  if (v > 0 && v < 0.01) return "<$0.01";
  return "$" + v.toFixed(2);
}

export function fmtTokensShort(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1_000_000) {
    const d = n >= 10_000_000 ? 0 : 1;
    return (n / 1_000_000).toFixed(d).replace(/\.0$/, "") + "M";
  }
  if (n >= 1000) return Math.round(n / 1000) + "K";
  return String(n);
}

export function fmtTokensFull(n: number | null | undefined): string {
  if (n == null) return "—";
  return n.toLocaleString("en-US");
}

export function totalTokens(t: Tokens): number {
  return t.input + t.output + t.cache_creation + t.cache_read;
}

export function fmtRelTime(ms: number | null | undefined, now = Date.now()): string {
  if (ms == null) return "—";
  const d = now - ms;
  const MIN = 60_000;
  const HR = 60 * MIN;
  if (d < MIN) return "just now";
  if (d < HR) return Math.round(d / MIN) + "m ago";
  if (d < 24 * HR) return Math.round(d / HR) + "h ago";
  return Math.round(d / (24 * HR)) + "d ago";
}

export function fmtClock(ms: number): string {
  const dt = new Date(ms);
  let h = dt.getHours();
  const m = dt.getMinutes().toString().padStart(2, "0");
  const ap = h >= 12 ? "PM" : "AM";
  h = h % 12 || 12;
  return `${h}:${m} ${ap}`;
}

export function fmtDuration(ms: number): string {
  const totalMin = Math.max(0, Math.round(ms / 60_000));
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  if (h <= 0) return `${m}m`;
  return `${h}h ${m.toString().padStart(2, "0")}m`;
}

// claude-opus-4-7 → opus 4.7
export function fmtModel(m: string | null | undefined): string {
  if (!m) return "—";
  const mm = m.match(/claude-(\w+)-(\d+)-(\d+)/);
  if (!mm) return m;
  return `${mm[1]} ${mm[2]}.${mm[3]}`;
}

export type ContextTier = "healthy" | "warning" | "high" | "critical";
export function contextTier(pct: number): ContextTier {
  if (pct >= 90) return "critical";
  if (pct >= 75) return "high";
  if (pct >= 50) return "warning";
  return "healthy";
}

export function repoName(cwd: string | null | undefined): string {
  if (!cwd) return "—";
  const parts = cwd.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? "—";
}

// Synthesize a 10-min-tick burn series from a single burn-rate number, so the
// sparkline has shape even before the backend exposes a real series.
export function synthBurnSeries(burn: number): number[] {
  if (burn <= 0) return [0, 0, 0, 0, 0, 0, 0];
  return [0, burn * 0.3, burn * 0.6, burn * 0.85, burn * 1.05, burn * 0.95, burn];
}
