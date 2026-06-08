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
    context_tokens: number;
    context_limit: number;
  };

  type BlockView = {
    start_ms: number;
    end_ms: number;
    actual_end_ms: number | null;
    is_active: boolean;
    is_gap: boolean;
    tokens: Tokens;
    message_count: number;
    models: string[];
    cost_usd: number;
    burn_usd_per_hr: number;
    projected_block_usd: number;
  };

  type Settings = { budget_window_usd: number };

  type Tab = "sessions" | "blocks" | "settings";

  let tab: Tab = $state("sessions");
  let sessions: Session[] = $state([]);
  let blocks: BlockView[] = $state([]);
  let settings: Settings = $state({ budget_window_usd: 5 });
  let loading = $state(true);
  let error: string | null = $state(null);
  let savedFlash = $state(false);
  let now = $state(Date.now());
  let showAllSessions = $state(false);
  let showAllBlocks = $state(false);
  let expandedId: string | null = $state(null);
  let hidden: Set<string> = $state(new Set());
  const DONE_BLOCKS_DEFAULT = 5;

  async function refreshHidden() {
    try {
      const ids = await invoke<string[]>("list_hidden");
      hidden = new Set(ids);
    } catch (e) {
      // non-fatal: hidden filter just doesn't apply
      console.error(e);
    }
  }

  async function hideSession(id: string) {
    await invoke("hide_session", { sessionId: id });
    expandedId = null;
    // data-changed event will reload; refresh hidden set proactively
    await refreshHidden();
  }

  async function openDashboard() {
    await invoke("open_dashboard");
  }

  function toggleExpand(id: string) {
    expandedId = expandedId === id ? null : id;
  }

  function relativeTime(ms: number | null): string {
    if (ms === null) return "—";
    const diff = Date.now() - ms;
    if (diff < 0) return "just now";
    const s = Math.floor(diff / 1000);
    if (s < 60) return `${s}s ago`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m} min ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    const d = Math.floor(h / 24);
    return `${d}d ago`;
  }

  async function load() {
    try {
      loading = true;
      error = null;
      const [s, b, st, h] = await Promise.all([
        invoke<Session[]>("list_sessions"),
        invoke<BlockView[]>("list_block_views"),
        invoke<Settings>("get_settings"),
        invoke<string[]>("list_hidden"),
      ]);
      sessions = s;
      blocks = b;
      settings = st;
      hidden = new Set(h);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function saveSettings() {
    try {
      await invoke("set_settings", { settings });
      savedFlash = true;
      setTimeout(() => (savedFlash = false), 1200);
    } catch (e) {
      error = String(e);
    }
  }

  let unlisten: UnlistenFn | undefined;
  let tickHandle: ReturnType<typeof setInterval> | undefined;
  onMount(async () => {
    await load();
    unlisten = await listen("data-changed", () => load());
    // Tick the "reset in" countdown for the active block every 30s.
    tickHandle = setInterval(() => (now = Date.now()), 30_000);
  });
  onDestroy(() => {
    unlisten?.();
    if (tickHandle) clearInterval(tickHandle);
  });

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
    if (c === 0) return "—";
    if (c < 0.01) return "<$0.01";
    return `$${c.toFixed(2)}`;
  }

  function repoName(cwd: string | null): string {
    if (!cwd) return "-";
    const parts = cwd.split("/").filter(Boolean);
    return parts[parts.length - 1] ?? "-";
  }

  function totalTokens(t: Tokens): number {
    return t.input + t.output + t.cache_creation;
  }

  function sessionTotal(s: Session): number {
    return totalTokens(s.tokens) + totalTokens(s.subagent_tokens);
  }

  function fmtTime(ms: number): string {
    const d = new Date(ms);
    return d.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  }

  function fmtDate(ms: number): string {
    const d = new Date(ms);
    return `${d.getMonth() + 1}/${d.getDate()}`;
  }

  function fmtDuration(ms: number): string {
    if (ms <= 0) return "0m";
    const mins = Math.floor(ms / 60_000);
    if (mins < 60) return `${mins}m`;
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m === 0 ? `${h}h` : `${h}h${m}m`;
  }

  function shortModel(m: string | null): string {
    if (!m) return "-";
    // claude-opus-4-7 -> opus-4-7
    return m.replace(/^claude-/, "");
  }

  function ctxPct(s: Session): number {
    if (!s.context_limit) return 0;
    return Math.min(100, (s.context_tokens / s.context_limit) * 100);
  }

  function ctxColor(pct: number): string {
    if (pct >= 90) return "#ef4444";
    if (pct >= 75) return "#f97316";
    if (pct >= 50) return "#facc15";
    return "#4ade80";
  }

  // Popover never shows soft-hidden sessions — dashboard is the place to
  // manage / unhide them.
  const shownSessions = $derived(sessions.filter((s) => !hidden.has(s.session_id)));
  const liveSessions = $derived(
    shownSessions.filter((s) => s.status === "active" || s.status === "idle"),
  );
  const inactiveCount = $derived(shownSessions.length - liveSessions.length);
  const visibleSessions = $derived(showAllSessions ? shownSessions : liveSessions);
  const totalCost = $derived(
    shownSessions.reduce((acc, s) => acc + (s.cost_usd ?? 0), 0),
  );
  // Names that appear on more than one visible session — we suffix those
  // with a short UID so the user can tell duplicates apart at a glance.
  const dupNames = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const s of visibleSessions) {
      if (s.name) counts.set(s.name, (counts.get(s.name) ?? 0) + 1);
    }
    const out = new Set<string>();
    for (const [n, c] of counts) if (c > 1) out.add(n);
    return out;
  });

  function displayName(s: Session): string {
    if (!s.name) return short(s.session_id);
    if (dupNames.has(s.name)) return `${s.name} #${short(s.session_id)}`;
    return s.name;
  }
  const activeBlock = $derived(blocks.find((b) => b.is_active));
  // Drop gap blocks (no usage data) and sort done blocks most-recent-first.
  const doneBlocks = $derived(
    blocks
      .filter((b) => !b.is_gap && !b.is_active)
      .sort((a, b) => b.start_ms - a.start_ms),
  );
  const visibleDoneBlocks = $derived(
    showAllBlocks ? doneBlocks : doneBlocks.slice(0, DONE_BLOCKS_DEFAULT),
  );
  const hiddenDoneCount = $derived(
    Math.max(0, doneBlocks.length - DONE_BLOCKS_DEFAULT),
  );
</script>

<main>
  <header>
    <div class="brand">
      <h1>Tokenscope</h1>
      <span class="stats">
        ${totalCost.toFixed(2)} total · {liveSessions.length} live · {shownSessions.length} sessions
      </span>
    </div>
    <div class="header-actions">
      <button class="icon-btn" onclick={openDashboard} title="Open Dashboard">⛶</button>
      <button class="icon-btn" onclick={load} disabled={loading} title="Refresh">↻</button>
    </div>
  </header>

  <nav class="tabs">
    <button class:active={tab === "sessions"} onclick={() => (tab = "sessions")}>Sessions</button>
    <button class:active={tab === "blocks"} onclick={() => (tab = "blocks")}>
      Blocks{activeBlock ? ` · $${activeBlock.cost_usd.toFixed(2)}` : ""}
    </button>
    <button class:active={tab === "settings"} onclick={() => (tab = "settings")}>Settings</button>
  </nav>

  {#if loading}
    <p class="empty">Loading…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if tab === "sessions"}
    {#if visibleSessions.length === 0}
      <p class="empty">
        {sessions.length === 0
          ? "No sessions found."
          : "No live sessions. Inactive only."}
        {#if !showAllSessions && inactiveCount > 0}
          <button class="link" onclick={() => (showAllSessions = true)}>
            Show {inactiveCount} inactive
          </button>
        {/if}
      </p>
    {:else}
      <ul class="session-list">
        {#each visibleSessions as s (s.session_id)}
          {@const open = expandedId === s.session_id}
          {@const pct = ctxPct(s)}
          <li class="session-card" class:open>
            <button
              type="button"
              class="card-trigger"
              aria-expanded={open}
              onclick={() => toggleExpand(s.session_id)}
            >
              <span class="dot {s.status}"></span>
              <div class="card-body">
                <div class="card-line">
                  <span class="name">{displayName(s)}</span>
                  <span class="cost">{fmtCost(s.cost_usd)}</span>
                </div>
                <div class="card-meta">
                  <span>{repoName(s.cwd)}</span>
                  <span class="sep">·</span>
                  <span class="mono">{shortModel(s.model)}</span>
                  {#if s.subagent_count > 0}
                    <span class="sep">·</span>
                    <span>{s.subagent_count} sub</span>
                  {/if}
                  <span class="caret">{open ? "▾" : "▸"}</span>
                </div>
                {#if s.context_tokens > 0 && s.status !== "inactive"}
                  <div class="ctx-row">
                    <div class="ctx-track">
                      <div
                        class="ctx-fill"
                        style="width: {pct}%; background: {ctxColor(pct)};"
                      ></div>
                    </div>
                    <span class="ctx-label" style="color: {ctxColor(pct)};">
                      {pct.toFixed(0)}% ctx
                    </span>
                  </div>
                {/if}
              </div>
            </button>
            {#if open}
              <dl class="card-detail">
                <dt>UID</dt><dd class="mono">{s.session_id}</dd>
                <dt>PID</dt><dd class="mono">{s.pid ?? "—"}</dd>
                <dt>cwd</dt><dd class="mono trunc">{s.cwd ?? "—"}</dd>
                <dt>model</dt><dd class="mono">{s.model ?? "—"}</dd>
                <dt>context</dt>
                <dd class="trunc">
                  {compact(s.context_tokens)} / {compact(s.context_limit)} ({pct.toFixed(1)}%)
                </dd>
                <dt>total in</dt><dd>{compact(s.tokens.input)}</dd>
                <dt>total out</dt><dd>{compact(s.tokens.output)}</dd>
                <dt>cache w</dt><dd>{compact(s.tokens.cache_creation)}</dd>
                <dt>cache r</dt><dd>{compact(s.tokens.cache_read)}</dd>
                {#if s.subagent_count > 0}
                  <dt>subagents</dt>
                  <dd>{s.subagent_count} ({compact(totalTokens(s.subagent_tokens))} tokens)</dd>
                {/if}
                <dt>updated</dt><dd>{relativeTime(s.updated_at_ms)}</dd>
              </dl>
              <div class="card-actions">
                <button class="link" onclick={() => hideSession(s.session_id)}>
                  Hide from popover
                </button>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
      {#if inactiveCount > 0}
        <div class="footer-row">
          <button
            class="link"
            onclick={() => (showAllSessions = !showAllSessions)}
          >
            {showAllSessions
              ? `Hide ${inactiveCount} inactive`
              : `Show ${inactiveCount} inactive`}
          </button>
        </div>
      {/if}
    {/if}
  {:else if tab === "blocks"}
    {#if !activeBlock && doneBlocks.length === 0}
      <p class="empty">No billing blocks yet.</p>
    {:else}
      {#if activeBlock}
        <section class="active-card">
          <div class="active-head">
            <span class="badge">ACTIVE 5H BLOCK</span>
            <span class="reset">resets in {fmtDuration(activeBlock.end_ms - now)}</span>
          </div>
          <div class="active-cost">${activeBlock.cost_usd.toFixed(2)}</div>
          {#if activeBlock.burn_usd_per_hr >= 0.01}
            <div class="active-burn">
              <span class="burn-rate">${activeBlock.burn_usd_per_hr.toFixed(2)}/hr</span>
              <span class="sep">·</span>
              <span class="proj">est. block ${activeBlock.projected_block_usd.toFixed(2)}</span>
            </div>
          {/if}
          <div class="active-meta">
            <span>{fmtTime(activeBlock.start_ms)}–{fmtTime(activeBlock.end_ms)}</span>
            <span>·</span>
            <span>{activeBlock.message_count} msgs</span>
            <span>·</span>
            <span>{compact(totalTokens(activeBlock.tokens))} tokens</span>
            <span>·</span>
            <span class="mono">{shortModel(activeBlock.models[0] ?? null)}</span>
          </div>
        </section>
      {/if}

      {#if doneBlocks.length > 0}
        <h2 class="section-title">Recent blocks</h2>
        <table>
          <thead>
            <tr>
              <th>WINDOW</th>
              <th class="num">MSGS</th><th class="num">TOTAL</th>
              <th class="num">COST</th><th>MODEL</th>
            </tr>
          </thead>
          <tbody>
            {#each visibleDoneBlocks as b (b.start_ms)}
              <tr>
                <td class="mono">
                  {fmtDate(b.start_ms)} {fmtTime(b.start_ms)}–{fmtTime(b.end_ms)}
                </td>
                <td class="num">{b.message_count}</td>
                <td class="num">{compact(totalTokens(b.tokens))}</td>
                <td class="num">{fmtCost(b.cost_usd)}</td>
                <td class="mono">{shortModel(b.models[0] ?? null)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if hiddenDoneCount > 0}
          <div class="footer-row">
            <button
              class="link"
              onclick={() => (showAllBlocks = !showAllBlocks)}
            >
              {showAllBlocks
                ? `Hide ${hiddenDoneCount} older`
                : `Show ${hiddenDoneCount} older`}
            </button>
          </div>
        {/if}
      {/if}
    {/if}
  {:else if tab === "settings"}
    <section class="settings">
      <label class="field">
        <span class="label">Budget alert threshold (USD per 5h block)</span>
        <span class="hint">
          Notify when the current billing block crosses this amount. Set to 0 to disable.
        </span>
        <div class="input-row">
          <input
            type="number"
            min="0"
            step="0.5"
            bind:value={settings.budget_window_usd}
          />
          <button onclick={saveSettings} class="save">
            {savedFlash ? "Saved ✓" : "Save"}
          </button>
        </div>
      </label>

      <div class="field">
        <span class="label">Pricing overrides</span>
        <span class="hint">
          Edit <code>~/.config/tokenscope/pricing.toml</code> to override per-million USD
          rates per model. Defaults follow Anthropic's published pricing.
        </span>
      </div>

      <div class="field">
        <span class="label">Data source</span>
        <span class="hint">
          Reads <code>~/.claude/sessions</code> and <code>~/.claude/projects</code>.
          Refreshes are filesystem-driven (~2s debounce).
        </span>
      </div>
    </section>
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
    padding: 14px 16px 16px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
    padding-bottom: 10px;
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

  .icon-btn,
  .save,
  .tabs button {
    background: #2a2a2a;
    border: 1px solid #3a3a3a;
    color: #e0e0e0;
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.15s;
  }
  .icon-btn:hover,
  .save:hover,
  .tabs button:hover {
    background: #3a3a3a;
  }
  .icon-btn:disabled,
  .save:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .tabs {
    display: flex;
    gap: 6px;
    margin-bottom: 12px;
  }
  .tabs button {
    flex: 1;
    padding: 6px 8px;
    font-size: 12px;
  }
  .tabs button.active {
    background: #3a3a3a;
    border-color: #4a4a4a;
    color: #fff;
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
    padding: 6px 8px;
    text-align: left;
    border-bottom: 1px solid #2a2a2a;
    white-space: nowrap;
    max-width: 220px;
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

  .settings {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding-top: 4px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .label {
    font-size: 12px;
    font-weight: 600;
    color: #ddd;
  }

  .hint {
    font-size: 11px;
    color: #888;
    line-height: 1.5;
  }

  .hint code {
    background: #2a2a2a;
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 10.5px;
  }

  .input-row {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }

  input[type="number"] {
    flex: 1;
    background: #0e0e0e;
    border: 1px solid #3a3a3a;
    color: #e0e0e0;
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  input[type="number"]:focus {
    outline: none;
    border-color: #4ade80;
  }

  .footer-row {
    text-align: center;
    padding: 10px 0 0;
  }

  .link {
    background: none;
    border: none;
    color: #888;
    font-size: 11px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 4px;
  }
  .link:hover {
    color: #ddd;
    background: #2a2a2a;
  }

  .active-card {
    background: linear-gradient(135deg, #1e2a1e 0%, #1a2a22 100%);
    border: 1px solid #2d4a2d;
    border-radius: 8px;
    padding: 14px 16px;
    margin-bottom: 14px;
  }

  .active-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }

  .badge {
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 1px;
    color: #4ade80;
    background: rgba(74, 222, 128, 0.12);
    padding: 3px 7px;
    border-radius: 4px;
  }

  .reset {
    font-size: 11px;
    color: #aaa;
    font-variant-numeric: tabular-nums;
  }

  .active-cost {
    font-size: 28px;
    font-weight: 700;
    color: #4ade80;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    margin: 4px 0 8px;
  }

  .active-burn {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .burn-rate {
    color: #facc15;
    font-weight: 600;
  }
  .proj {
    color: #ccc;
  }

  .active-meta {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    font-size: 11px;
    color: #999;
    font-variant-numeric: tabular-nums;
  }

  .section-title {
    margin: 4px 0 8px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
    font-weight: 600;
  }

  .session-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .session-card {
    border-radius: 6px;
    transition: background 0.12s;
  }
  .session-card.open {
    background: #232323;
  }

  .card-trigger {
    width: 100%;
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    text-align: left;
    padding: 8px 10px;
    border-radius: 6px;
    display: grid;
    grid-template-columns: 14px 1fr;
    gap: 8px;
    align-items: start;
    font: inherit;
  }
  .card-trigger:hover {
    background: #232323;
  }
  .session-card.open .card-trigger:hover {
    background: transparent;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-top: 6px;
    background: #555;
  }
  .dot.active {
    background: #4ade80;
    box-shadow: 0 0 6px rgba(74, 222, 128, 0.5);
  }
  .dot.idle {
    background: #facc15;
  }
  .dot.inactive {
    background: #555;
  }

  .card-body {
    min-width: 0;
  }

  .card-line {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }

  .name {
    font-size: 13px;
    font-weight: 500;
    color: #e8e8e8;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .cost {
    font-size: 13px;
    font-weight: 600;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .card-meta {
    margin-top: 2px;
    font-size: 11px;
    color: #888;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    gap: 5px;
    align-items: center;
    min-width: 0;
  }

  .sep {
    color: #555;
  }

  .caret {
    margin-left: auto;
    color: #666;
    font-size: 10px;
  }

  .card-detail {
    margin: 0;
    padding: 8px 12px 12px 32px;
    display: grid;
    grid-template-columns: 60px 1fr 60px 1fr;
    gap: 4px 10px;
    border-top: 1px solid #2a2a2a;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .card-detail dt {
    color: #777;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-size: 9.5px;
    align-self: center;
  }
  .card-detail dd {
    margin: 0;
    color: #ddd;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-detail .trunc {
    grid-column: 2 / -1;
  }

  .ctx-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 5px;
  }

  .ctx-track {
    flex: 1;
    height: 4px;
    background: #2a2a2a;
    border-radius: 2px;
    overflow: hidden;
    min-width: 0;
  }

  .ctx-fill {
    height: 100%;
    border-radius: 2px;
    transition: width 0.2s, background 0.2s;
  }

  .ctx-label {
    font-size: 10px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .header-actions {
    display: flex;
    gap: 6px;
  }

  .card-actions {
    padding: 0 12px 10px 32px;
    display: flex;
    gap: 6px;
  }
</style>
