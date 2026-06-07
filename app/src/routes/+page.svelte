<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";

  type Tokens = {
    input: number;
    output: number;
    cache_creation: number;
    cache_read: number;
  };

  type Session = {
    session_id: string;
    name: string | null;
    cwd: string | null;
    pid: number | null;
    status: "active" | "idle" | "inactive";
    tokens: Tokens;
    subagent_tokens: Tokens;
    subagent_count: number;
    model: string | null;
    cost_usd: number | null;
    updated_at_ms: number | null;
  };

  let sessions: Session[] = $state([]);
  let loading = $state(true);
  let error: string | null = $state(null);

  async function load() {
    try {
      loading = true;
      error = null;
      sessions = await invoke<Session[]>("list_sessions");
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  let unlisten: UnlistenFn | undefined;
  onMount(async () => {
    await load();
    unlisten = await listen("data-changed", () => load());
  });
  onDestroy(() => unlisten?.());

  function short(id: string): string {
    return id.slice(0, 8);
  }

  function compact(n: number): string {
    if (n < 1_000) return String(n);
    if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}K`;
    if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    return `${(n / 1_000_000_000).toFixed(1)}B`;
  }

  function fmtCost(c: number | null): string {
    if (c === null) return "N/A";
    if (c < 0.01) return "<$0.01";
    return `$${c.toFixed(2)}`;
  }

  function repoName(cwd: string | null): string {
    if (!cwd) return "-";
    const parts = cwd.split("/").filter(Boolean);
    return parts[parts.length - 1] ?? "-";
  }

  function totalTokens(s: Session): number {
    return (
      s.tokens.input +
      s.tokens.output +
      s.tokens.cache_creation +
      s.subagent_tokens.input +
      s.subagent_tokens.output +
      s.subagent_tokens.cache_creation
    );
  }

  const totalCost = $derived(
    sessions.reduce((acc, s) => acc + (s.cost_usd ?? 0), 0),
  );
  const activeCount = $derived(
    sessions.filter((s) => s.status === "active" || s.status === "idle").length,
  );
</script>

<main>
  <header>
    <div class="brand">
      <h1>cc-monitor</h1>
      <span class="stats">
        ${totalCost.toFixed(2)} total · {activeCount} live · {sessions.length} sessions
      </span>
    </div>
    <button onclick={load} disabled={loading}>↻</button>
  </header>

  {#if loading}
    <p class="empty">Loading…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if sessions.length === 0}
    <p class="empty">No sessions found.</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th>UID</th><th>NAME</th><th>REPO</th><th>STATUS</th>
          <th class="num">TOTAL</th><th class="num">SUBS</th>
          <th class="num">COST</th><th>MODEL</th>
        </tr>
      </thead>
      <tbody>
        {#each sessions as s (s.session_id)}
          <tr class={s.status}>
            <td class="mono">{short(s.session_id)}</td>
            <td>{s.name ?? "-"}</td>
            <td>{repoName(s.cwd)}</td>
            <td>{s.status}</td>
            <td class="num">{compact(totalTokens(s))}</td>
            <td class="num">{s.subagent_count}</td>
            <td class="num">{fmtCost(s.cost_usd)}</td>
            <td class="mono">{s.model ?? "-"}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</main>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    background: #1a1a1a;
    color: #e0e0e0;
    font-family:
      -apple-system,
      BlinkMacSystemFont,
      "SF Pro Text",
      sans-serif;
    font-size: 13px;
    line-height: 1.4;
  }

  main {
    padding: 16px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
    padding-bottom: 12px;
    border-bottom: 1px solid #2a2a2a;
  }

  .brand {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  .stats {
    font-size: 11px;
    color: #888;
    font-variant-numeric: tabular-nums;
  }

  button {
    background: #2a2a2a;
    border: 1px solid #3a3a3a;
    color: #e0e0e0;
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 14px;
    transition: background 0.15s;
  }
  button:hover {
    background: #3a3a3a;
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .empty,
  .error {
    text-align: center;
    color: #888;
    padding: 32px;
  }
  .error {
    color: #ff6b6b;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }

  th,
  td {
    padding: 6px 10px;
    text-align: left;
    border-bottom: 1px solid #2a2a2a;
    white-space: nowrap;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  th {
    font-weight: 600;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
  }

  th.num,
  td.num {
    text-align: right;
  }

  .mono {
    font-family: "SF Mono", Menlo, monospace;
    font-size: 11px;
  }

  tr.active td {
    color: #4ade80;
  }
  tr.idle td {
    color: #facc15;
  }
  tr.inactive td {
    color: #aaa;
  }
</style>
