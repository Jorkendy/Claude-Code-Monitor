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

  type Repo = {
    repo: string;
    session_count: number;
    live_count: number;
    total_cost_usd: number;
    total_tokens: number;
    top_model: string | null;
  };

  type Tab = "sessions" | "repos";
  type SortKey =
    | "name"
    | "repo"
    | "status"
    | "tokens"
    | "context"
    | "cost"
    | "model"
    | "updated";
  type SortDir = "asc" | "desc";

  let tab: Tab = $state("sessions");
  let sessions: Session[] = $state([]);
  let repos: Repo[] = $state([]);
  let hidden: Set<string> = $state(new Set());
  let loading = $state(true);
  let error: string | null = $state(null);

  let search = $state("");
  let statusFilter: "all" | "live" | "inactive" = $state("all");
  let repoFilter: string = $state("all");
  let showHidden = $state(false);
  let sortKey: SortKey = $state("cost");
  let sortDir: SortDir = $state("desc");
  let expandedId: string | null = $state(null);
  let confirmDeleteId: string | null = $state(null);

  // `loading` flips only on the initial mount load — subsequent refreshes
  // (after hide/delete or watcher events) run in the background so the
  // table doesn't unmount and the scroll position survives.
  async function load(showSpinner = false) {
    try {
      if (showSpinner) loading = true;
      error = null;
      const [s, r, h] = await Promise.all([
        invoke<Session[]>("list_sessions"),
        invoke<Repo[]>("list_repo_rollups"),
        invoke<string[]>("list_hidden"),
      ]);
      sessions = s;
      repos = r;
      hidden = new Set(h);
    } catch (e) {
      error = String(e);
    } finally {
      if (showSpinner) loading = false;
    }
  }

  async function hideSession(id: string) {
    try {
      // Optimistic: add to local hidden set immediately so the row either
      // disappears (default view) or fades (show-hidden view) without a
      // full re-render that would reset scroll.
      hidden = new Set([...hidden, id]);
      await invoke("hide_session", { sessionId: id });
      await load();
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  async function unhideSession(id: string) {
    try {
      const next = new Set(hidden);
      next.delete(id);
      hidden = next;
      await invoke("unhide_session", { sessionId: id });
      await load();
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  async function unhideAll() {
    try {
      hidden = new Set();
      await invoke("unhide_all");
      await load();
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  async function deleteSession(id: string) {
    try {
      // Optimistic: drop the row from local state so it vanishes in place.
      sessions = sessions.filter((s) => s.session_id !== id);
      confirmDeleteId = null;
      await invoke("delete_session_files", { sessionId: id });
      await load();
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  let unlisten: UnlistenFn | undefined;
  onMount(async () => {
    await load(true);
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
    if (c === 0) return "—";
    if (c < 0.01) return "<$0.01";
    return `$${c.toFixed(2)}`;
  }

  function repoName(cwd: string | null): string {
    if (!cwd) return "-";
    const parts = cwd.split("/").filter(Boolean);
    return parts[parts.length - 1] ?? "-";
  }

  function sessionTotal(s: Session): number {
    return (
      s.tokens.input +
      s.tokens.output +
      s.tokens.cache_creation +
      s.subagent_tokens.input +
      s.subagent_tokens.output +
      s.subagent_tokens.cache_creation
    );
  }

  function shortModel(m: string | null): string {
    if (!m) return "-";
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

  function fmtDateTime(ms: number | null): string {
    if (ms === null) return "—";
    const d = new Date(ms);
    return `${d.getMonth() + 1}/${d.getDate()} ${d
      .toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false })}`;
  }

  const allRepos = $derived(
    Array.from(new Set(sessions.map((s) => repoName(s.cwd)))).sort(),
  );

  function statusMatch(s: Session): boolean {
    if (statusFilter === "all") return true;
    if (statusFilter === "live") return s.status === "active" || s.status === "idle";
    return s.status === "inactive";
  }

  function searchMatch(s: Session): boolean {
    if (!search.trim()) return true;
    const q = search.trim().toLowerCase();
    return (
      (s.name?.toLowerCase().includes(q) ?? false) ||
      s.session_id.toLowerCase().includes(q) ||
      (s.cwd?.toLowerCase().includes(q) ?? false) ||
      (s.model?.toLowerCase().includes(q) ?? false)
    );
  }

  function hiddenMatch(s: Session): boolean {
    return showHidden ? hidden.has(s.session_id) : !hidden.has(s.session_id);
  }

  function repoMatch(s: Session): boolean {
    if (repoFilter === "all") return true;
    return repoName(s.cwd) === repoFilter;
  }

  function compare(a: Session, b: Session): number {
    let av: number | string;
    let bv: number | string;
    switch (sortKey) {
      case "name":
        av = (a.name ?? a.session_id).toLowerCase();
        bv = (b.name ?? b.session_id).toLowerCase();
        break;
      case "repo":
        av = repoName(a.cwd).toLowerCase();
        bv = repoName(b.cwd).toLowerCase();
        break;
      case "status":
        av = a.status;
        bv = b.status;
        break;
      case "tokens":
        av = sessionTotal(a);
        bv = sessionTotal(b);
        break;
      case "context":
        av = ctxPct(a);
        bv = ctxPct(b);
        break;
      case "cost":
        av = a.cost_usd ?? 0;
        bv = b.cost_usd ?? 0;
        break;
      case "model":
        av = (a.model ?? "").toLowerCase();
        bv = (b.model ?? "").toLowerCase();
        break;
      case "updated":
        av = a.updated_at_ms ?? 0;
        bv = b.updated_at_ms ?? 0;
        break;
    }
    const cmp = av < bv ? -1 : av > bv ? 1 : a.session_id.localeCompare(b.session_id);
    return sortDir === "desc" ? -cmp : cmp;
  }

  const visible = $derived(
    sessions
      .filter(hiddenMatch)
      .filter(statusMatch)
      .filter(repoMatch)
      .filter(searchMatch)
      .slice()
      .sort(compare),
  );

  function setSort(k: SortKey) {
    if (sortKey === k) {
      sortDir = sortDir === "desc" ? "asc" : "desc";
    } else {
      sortKey = k;
      sortDir = k === "name" || k === "repo" || k === "model" ? "asc" : "desc";
    }
  }

  function sortIndicator(k: SortKey): string {
    if (sortKey !== k) return "";
    return sortDir === "desc" ? " ▾" : " ▴";
  }

  const totalCost = $derived(visible.reduce((acc, s) => acc + (s.cost_usd ?? 0), 0));
  const hiddenCount = $derived(hidden.size);
</script>

<main>
  <header>
    <div>
      <h1>Tokenscope Dashboard</h1>
      <span class="stats">
        {visible.length} of {sessions.length} sessions · ${totalCost.toFixed(2)} total
        {#if hiddenCount > 0}
          · {hiddenCount} hidden
        {/if}
      </span>
    </div>
    <button class="icon-btn" onclick={() => load(true)} disabled={loading} title="Refresh">↻</button>
  </header>

  <nav class="tabs">
    <button class:active={tab === "sessions"} onclick={() => (tab = "sessions")}>
      Sessions
    </button>
    <button class:active={tab === "repos"} onclick={() => (tab = "repos")}>
      Repos ({repos.length})
    </button>
  </nav>

  {#if loading}
    <p class="empty">Loading…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if tab === "sessions"}
    <div class="filters">
      <input
        class="search"
        type="search"
        placeholder="Search by name, UID, cwd, model…"
        bind:value={search}
      />
      <select bind:value={statusFilter}>
        <option value="all">All status</option>
        <option value="live">Live (active+idle)</option>
        <option value="inactive">Inactive</option>
      </select>
      <select bind:value={repoFilter}>
        <option value="all">All repos</option>
        {#each allRepos as r}
          <option value={r}>{r}</option>
        {/each}
      </select>
      <label class="chk">
        <input type="checkbox" bind:checked={showHidden} />
        Show hidden ({hiddenCount})
      </label>
      {#if showHidden && hiddenCount > 0}
        <button class="link" onclick={unhideAll}>Unhide all</button>
      {/if}
    </div>

    {#if visible.length === 0}
      <p class="empty">No sessions match.</p>
    {:else}
      <table class="grid">
        <thead>
          <tr>
            <th class="sortable" onclick={() => setSort("name")}>
              NAME{sortIndicator("name")}
            </th>
            <th class="sortable" onclick={() => setSort("repo")}>
              REPO{sortIndicator("repo")}
            </th>
            <th class="sortable" onclick={() => setSort("status")}>
              STATUS{sortIndicator("status")}
            </th>
            <th class="num sortable" onclick={() => setSort("tokens")}>
              TOKENS{sortIndicator("tokens")}
            </th>
            <th class="num sortable" onclick={() => setSort("context")}>
              CTX%{sortIndicator("context")}
            </th>
            <th class="num sortable" onclick={() => setSort("cost")}>
              COST{sortIndicator("cost")}
            </th>
            <th class="sortable" onclick={() => setSort("model")}>
              MODEL{sortIndicator("model")}
            </th>
            <th class="sortable" onclick={() => setSort("updated")}>
              UPDATED{sortIndicator("updated")}
            </th>
            <th>SUBS</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each visible as s (s.session_id)}
            {@const pct = ctxPct(s)}
            {@const isHidden = hidden.has(s.session_id)}
            <tr class={s.status} class:hidden-row={isHidden}>
              <td>
                <button
                  type="button"
                  class="row-toggle"
                  onclick={() =>
                    (expandedId = expandedId === s.session_id ? null : s.session_id)}
                  title={s.session_id}
                >
                  <span class="dot {s.status}"></span>
                  {s.name ?? short(s.session_id)}
                </button>
              </td>
              <td>{repoName(s.cwd)}</td>
              <td>{s.status}</td>
              <td class="num">{compact(sessionTotal(s))}</td>
              <td class="num">
                {#if s.context_tokens > 0 && s.status !== "inactive"}
                  <span style="color: {ctxColor(pct)};">{pct.toFixed(0)}%</span>
                {:else}
                  —
                {/if}
              </td>
              <td class="num">{fmtCost(s.cost_usd)}</td>
              <td class="mono">{shortModel(s.model)}</td>
              <td class="mono">{fmtDateTime(s.updated_at_ms)}</td>
              <td class="num">
                {s.subagent_count > 0 ? s.subagent_count : "—"}
              </td>
              <td class="actions">
                {#if isHidden}
                  <button
                    class="btn-ghost"
                    onclick={() => unhideSession(s.session_id)}
                  >
                    Unhide
                  </button>
                {:else}
                  <button class="btn-ghost" onclick={() => hideSession(s.session_id)}>
                    Hide
                  </button>
                {/if}
                {#if s.status === "inactive"}
                  {#if confirmDeleteId === s.session_id}
                    <button
                      class="btn-danger"
                      onclick={() => deleteSession(s.session_id)}
                    >
                      Confirm delete
                    </button>
                    <button class="btn-ghost" onclick={() => (confirmDeleteId = null)}>
                      Cancel
                    </button>
                  {:else}
                    <button
                      class="btn-ghost"
                      onclick={() => (confirmDeleteId = s.session_id)}
                    >
                      Delete
                    </button>
                  {/if}
                {/if}
              </td>
            </tr>
            {#if expandedId === s.session_id}
              <tr class="detail-row">
                <td colspan="10">
                  <dl class="detail">
                    <dt>UID</dt><dd class="mono">{s.session_id}</dd>
                    <dt>PID</dt><dd class="mono">{s.pid ?? "—"}</dd>
                    <dt>cwd</dt><dd class="mono">{s.cwd ?? "—"}</dd>
                    <dt>model</dt><dd class="mono">{s.model ?? "—"}</dd>
                    <dt>context</dt>
                    <dd>
                      {compact(s.context_tokens)} / {compact(s.context_limit)} ({pct.toFixed(1)}%)
                    </dd>
                    <dt>main tokens</dt>
                    <dd>
                      in {compact(s.tokens.input)} · out {compact(s.tokens.output)} ·
                      cache w {compact(s.tokens.cache_creation)} · cache r {compact(
                        s.tokens.cache_read,
                      )}
                    </dd>
                    {#if s.subagent_count > 0}
                      <dt>subagent tokens</dt>
                      <dd>
                        {s.subagent_count} agents · in {compact(s.subagent_tokens.input)}
                        · out {compact(s.subagent_tokens.output)} · cache w {compact(
                          s.subagent_tokens.cache_creation,
                        )} · cache r {compact(s.subagent_tokens.cache_read)}
                      </dd>
                    {/if}
                  </dl>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}
  {:else if tab === "repos"}
    {#if repos.length === 0}
      <p class="empty">No repos.</p>
    {:else}
      <table class="grid">
        <thead>
          <tr>
            <th>REPO</th>
            <th class="num">SESSIONS</th>
            <th class="num">LIVE</th>
            <th class="num">TOKENS</th>
            <th class="num">TOTAL COST</th>
            <th>TOP MODEL</th>
          </tr>
        </thead>
        <tbody>
          {#each repos as r (r.repo)}
            <tr>
              <td>{r.repo}</td>
              <td class="num">{r.session_count}</td>
              <td class="num">{r.live_count}</td>
              <td class="num">{compact(r.total_tokens)}</td>
              <td class="num">{fmtCost(r.total_cost_usd)}</td>
              <td class="mono">{shortModel(r.top_model)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</main>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    background: #141414;
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
    padding: 18px 22px;
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: 14px;
    padding-bottom: 12px;
    border-bottom: 1px solid #2a2a2a;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
  }
  .stats {
    font-size: 12px;
    color: #888;
    font-variant-numeric: tabular-nums;
  }

  .icon-btn,
  .tabs button,
  .filters select,
  .filters input.search,
  .btn-ghost,
  .btn-danger,
  .link {
    background: #2a2a2a;
    border: 1px solid #3a3a3a;
    color: #e0e0e0;
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.12s, border 0.12s;
  }
  .icon-btn:hover,
  .btn-ghost:hover {
    background: #3a3a3a;
  }
  .icon-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .tabs {
    display: flex;
    gap: 6px;
    margin-bottom: 14px;
  }
  .tabs button.active {
    background: #3a3a3a;
    border-color: #4a4a4a;
    color: #fff;
  }

  .filters {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
    margin-bottom: 12px;
  }
  .filters .search {
    flex: 1;
    min-width: 220px;
    cursor: text;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: #aaa;
    cursor: pointer;
  }
  .link {
    background: none;
    border: none;
    color: #888;
    font-size: 11px;
    padding: 4px 8px;
  }
  .link:hover {
    color: #ddd;
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

  table.grid {
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }
  th,
  td {
    padding: 7px 10px;
    text-align: left;
    border-bottom: 1px solid #1f1f1f;
    white-space: nowrap;
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  th {
    font-weight: 600;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
    background: #141414;
    position: sticky;
    top: 0;
  }
  th.sortable {
    cursor: pointer;
    user-select: none;
  }
  th.sortable:hover {
    color: #ddd;
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
    color: #d8f5d8;
  }
  tr.idle td {
    color: #f5ecd0;
  }
  tr.inactive td {
    color: #aaa;
  }
  tr.hidden-row td {
    opacity: 0.45;
  }

  .row-toggle {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    padding: 0;
    font: inherit;
    text-align: left;
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #555;
    flex-shrink: 0;
  }
  .dot.active {
    background: #4ade80;
  }
  .dot.idle {
    background: #facc15;
  }
  .dot.inactive {
    background: #555;
  }

  .actions {
    display: flex;
    gap: 4px;
    justify-content: flex-end;
  }
  .btn-ghost {
    padding: 4px 10px;
    font-size: 11px;
  }
  .btn-danger {
    padding: 4px 10px;
    font-size: 11px;
    background: #7c1d1d;
    border-color: #a02b2b;
    color: #ffe4e4;
  }
  .btn-danger:hover {
    background: #a02b2b;
  }

  tr.detail-row td {
    background: #1a1a1a;
    padding: 14px 22px;
  }
  .detail {
    margin: 0;
    display: grid;
    grid-template-columns: 100px 1fr;
    gap: 4px 14px;
    font-size: 12px;
  }
  .detail dt {
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-size: 10px;
    align-self: center;
  }
  .detail dd {
    margin: 0;
    color: #ddd;
    white-space: normal;
    word-break: break-all;
  }
</style>
