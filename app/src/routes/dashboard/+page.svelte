<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";
  import {
    fmtUSD,
    fmtTokensShort,
    fmtTokensFull,
    totalTokens,
    fmtRelTime,
    fmtModel,
    contextTier,
    repoName,
    type Tokens,
  } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import StatusDot from "$lib/components/StatusDot.svelte";
  import ContextBar from "$lib/components/ContextBar.svelte";
  import CopyButton from "$lib/components/CopyButton.svelte";

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
    total_tokens: number;
    total_cost_usd: number;
    top_model: string | null;
  };

  type View = "sessions" | "repos";
  type StatusFilter = "all" | "active" | "idle" | "inactive";
  type Plan = "api" | "pro" | "max-5x" | "max-20x";
  type Settings = {
    budget_window_usd: number;
    plan: Plan;
    rate_limit_warn_pct: number;
  };
  const QUOTA: Record<Plan, number | null> = {
    api: null,
    pro: 45,
    "max-5x": 225,
    "max-20x": 900,
  };
  type SessionSortKey =
    | "name"
    | "repo"
    | "model"
    | "context"
    | "tokens"
    | "cost_usd"
    | "subagent_count"
    | "updated_at_ms";
  type RepoSortKey = keyof Repo;
  type SortDir = "asc" | "desc";

  // --- state -----------------------------------------------------------
  let view: View = $state("sessions");
  let sessions: Session[] = $state([]);
  let repos: Repo[] = $state([]);
  let hidden: Set<string> = $state(new Set());
  let settings: Settings = $state({
    budget_window_usd: 0,
    plan: "api",
    rate_limit_warn_pct: 90,
  });
  let loading = $state(true);
  let error: string | null = $state(null);
  let now = $state(Date.now());
  const isSubs = $derived(QUOTA[settings.plan] != null);

  // sessions toolbar/sort state
  let search = $state("");
  let statusFilter: StatusFilter = $state("all");
  let repoFilter: string = $state("all");
  let showHidden = $state(false);
  let sortKey: SessionSortKey = $state("updated_at_ms");
  let sortDir: SortDir = $state("desc");
  let openId: string | null = $state(null);
  let confirm: Session | null = $state(null);

  // repos sort state
  let repoSortKey: RepoSortKey = $state("total_cost_usd");
  let repoSortDir: SortDir = $state("desc");

  async function load(showSpinner = false) {
    try {
      if (showSpinner) loading = true;
      error = null;
      const [s, r, h, st] = await Promise.all([
        invoke<Session[]>("list_sessions"),
        invoke<Repo[]>("list_repo_rollups"),
        invoke<string[]>("list_hidden"),
        invoke<Settings>("get_settings"),
      ]);
      sessions = s;
      repos = r;
      settings = st;
      // Drop hidden ids whose session is gone, so the badge counter and the
      // file on disk don't slowly fill with ghosts.
      const aliveIds = s.map((x) => x.session_id);
      const aliveSet = new Set(aliveIds);
      if (h.some((id) => !aliveSet.has(id))) {
        await invoke("prune_hidden", { aliveIds });
        hidden = new Set(h.filter((id) => aliveSet.has(id)));
      } else {
        hidden = new Set(h);
      }
    } catch (e) {
      error = String(e);
    } finally {
      if (showSpinner) loading = false;
    }
  }

  async function toggleHide(s: Session) {
    const id = s.session_id;
    const wasHidden = hidden.has(id);
    try {
      const next = new Set(hidden);
      if (wasHidden) next.delete(id);
      else next.add(id);
      hidden = next;
      await invoke(wasHidden ? "unhide_session" : "hide_session", {
        sessionId: id,
      });
      await load();
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  async function deleteSession(s: Session) {
    try {
      sessions = sessions.filter((x) => x.session_id !== s.session_id);
      confirm = null;
      await invoke("delete_session_files", { sessionId: s.session_id });
      await load();
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  let unlisten: UnlistenFn | undefined;
  let tickHandle: ReturnType<typeof setInterval> | undefined;
  onMount(async () => {
    await load(true);
    unlisten = await listen("data-changed", () => load(false));
    tickHandle = setInterval(() => (now = Date.now()), 30_000);
  });
  onDestroy(() => {
    unlisten?.();
    if (tickHandle) clearInterval(tickHandle);
  });

  // --- derived ---------------------------------------------------------
  const repoOptions = $derived([
    "all",
    ...Array.from(new Set(sessions.map((s) => repoName(s.cwd)))).sort(),
  ]);

  function sessionVal(s: Session, k: SessionSortKey): number | string {
    switch (k) {
      case "name":
        return (s.name || s.session_id).toLowerCase();
      case "repo":
        return repoName(s.cwd).toLowerCase();
      case "model":
        return (s.model || "").toLowerCase();
      case "context":
        return s.context_tokens / s.context_limit;
      case "tokens":
        return totalTokens(s.tokens);
      case "cost_usd":
        return s.cost_usd ?? -1;
      case "subagent_count":
        return s.subagent_count;
      case "updated_at_ms":
        return s.updated_at_ms ?? 0;
    }
  }

  const filteredSessions = $derived.by(() => {
    let r = sessions.slice();
    if (!showHidden) r = r.filter((s) => !hidden.has(s.session_id));
    if (statusFilter !== "all") r = r.filter((s) => s.status === statusFilter);
    if (repoFilter !== "all")
      r = r.filter((s) => repoName(s.cwd) === repoFilter);
    if (search.trim()) {
      const q = search.toLowerCase();
      r = r.filter(
        (s) =>
          (s.name || "").toLowerCase().includes(q) ||
          s.session_id.toLowerCase().includes(q) ||
          (s.cwd || "").toLowerCase().includes(q),
      );
    }
    r.sort((a, b) => {
      const av = sessionVal(a, sortKey);
      const bv = sessionVal(b, sortKey);
      const cmp =
        typeof av === "string" && typeof bv === "string"
          ? av.localeCompare(bv)
          : (av as number) - (bv as number);
      return sortDir === "asc" ? cmp : -cmp;
    });
    return r;
  });

  const liveCount = $derived(
    sessions.filter((s) => s.status === "active" || s.status === "idle").length,
  );

  function onSortSessions(k: SessionSortKey) {
    if (k === sortKey) sortDir = sortDir === "asc" ? "desc" : "asc";
    else {
      sortKey = k;
      sortDir = k === "name" || k === "repo" || k === "model" ? "asc" : "desc";
    }
  }

  function repoVal(r: Repo, k: RepoSortKey): number | string {
    const v = r[k];
    if (typeof v === "string") return v.toLowerCase();
    return (v as number) ?? 0;
  }

  const sortedRepos = $derived.by(() => {
    const r = repos.slice();
    r.sort((a, b) => {
      const av = repoVal(a, repoSortKey);
      const bv = repoVal(b, repoSortKey);
      const cmp =
        typeof av === "string" && typeof bv === "string"
          ? av.localeCompare(bv)
          : (av as number) - (bv as number);
      return repoSortDir === "asc" ? cmp : -cmp;
    });
    return r;
  });
  const maxRepoCost = $derived(
    Math.max(...repos.map((r) => r.total_cost_usd), 0.0001),
  );
  const maxRepoTokens = $derived(
    Math.max(...repos.map((r) => r.total_tokens), 1),
  );
  const grandCost = $derived(repos.reduce((s, r) => s + r.total_cost_usd, 0));
  const grandTokens = $derived(repos.reduce((s, r) => s + r.total_tokens, 0));

  function onSortRepos(k: RepoSortKey) {
    if (k === repoSortKey) repoSortDir = repoSortDir === "asc" ? "desc" : "asc";
    else {
      repoSortKey = k;
      repoSortDir = k === "repo" || k === "top_model" ? "asc" : "desc";
    }
  }

  const sessionCols: Array<{
    k: SessionSortKey | "status" | "actions";
    label: string;
    align: "left" | "right";
    sort: boolean;
  }> = [
    { k: "status", label: "", align: "left", sort: false },
    { k: "name", label: "Session", align: "left", sort: true },
    { k: "repo", label: "Repo", align: "left", sort: true },
    { k: "model", label: "Model", align: "left", sort: true },
    { k: "context", label: "Context", align: "left", sort: true },
    { k: "tokens", label: "Tokens", align: "right", sort: true },
    { k: "cost_usd", label: "Cost", align: "right", sort: true },
    { k: "subagent_count", label: "Sub", align: "right", sort: true },
    { k: "updated_at_ms", label: "Updated", align: "right", sort: true },
    { k: "actions", label: "", align: "right", sort: false },
  ];

  const repoCols = $derived<
    Array<{ k: RepoSortKey; label: string; align: "left" | "right" }>
  >([
    { k: "repo", label: "Repository", align: "left" },
    { k: "session_count", label: "Sessions", align: "right" },
    { k: "live_count", label: "Live", align: "right" },
    { k: "total_tokens", label: "Tokens", align: "right" },
    {
      k: "total_cost_usd",
      label: isSubs ? "Volume" : "Total cost",
      align: "right",
    },
    { k: "top_model", label: "Top model", align: "left" },
  ]);
</script>

<div class="dash-win">
  <div class="dash-body">
    <aside class="dash-sidebar">
      <div class="dash-sb-section">MONITOR</div>
      <button
        class="dash-navitem"
        class:is-on={view === "sessions"}
        onclick={() => (view = "sessions")}
      >
        <Icon name="list" size={15} />
        <span>Sessions</span>
        <span class="dash-nav-count ts-tnum">{sessions.length}</span>
      </button>
      <button
        class="dash-navitem"
        class:is-on={view === "repos"}
        onclick={() => (view = "repos")}
      >
        <Icon name="layers" size={15} />
        <span>Repositories</span>
        <span class="dash-nav-count ts-tnum">{repos.length}</span>
      </button>

      <div class="dash-sb-spacer"></div>

      <div class="dash-sb-foot">
        <div class="dash-sb-live ts-tnum">
          <span class="dash-livepip"></span>
          {liveCount} session{liveCount === 1 ? "" : "s"} live
        </div>
        <div class="dash-sb-path ts-mono">~/.claude/</div>
      </div>
    </aside>

    <main class="dash-main">
      <div class="ts-dashbody">
        {#if loading}
          <div class="ts-empty">Loading…</div>
        {:else if error}
          <div class="ts-empty ts-err">{error}</div>
        {:else if view === "sessions"}
          <div class="ts-dash-h1row">
            <h1 class="ts-dash-h1">Sessions</h1>
            <span class="ts-dash-sub ts-tnum">
              {filteredSessions.length} shown · {liveCount} live
            </span>
          </div>

          <div class="ts-toolbar">
            <div class="ts-searchbox">
              <span class="ts-search-ico"><Icon name="search" size={14} /></span>
              <input
                class="ts-search-input"
                type="text"
                placeholder="Search name, path, or UID…"
                bind:value={search}
              />
              {#if search}
                <button class="ts-search-clear" onclick={() => (search = "")}>
                  <Icon name="x" size={13} />
                </button>
              {/if}
            </div>

            <div class="ts-segmented">
              {#each ["all", "active", "idle", "inactive"] as const as st}
                <button
                  class="ts-seg"
                  class:is-on={statusFilter === st}
                  onclick={() => (statusFilter = st)}
                >
                  {#if st !== "all"}
                    <StatusDot status={st} size={6} />
                  {/if}
                  {st}
                </button>
              {/each}
            </div>

            <select class="ts-select" bind:value={repoFilter}>
              {#each repoOptions as r}
                <option value={r}>{r === "all" ? "All repos" : r}</option>
              {/each}
            </select>

            <button
              class="ts-ghostbtn"
              class:is-on={showHidden}
              onclick={() => (showHidden = !showHidden)}
            >
              <Icon name={showHidden ? "eye" : "eyeOff"} size={14} />
              {hidden.size > 0 ? `${hidden.size} hidden` : "hidden"}
            </button>
          </div>

          <div class="ts-table ts-sesstable">
            <div class="ts-tr ts-tr-head ts-sess-grid">
              {#each sessionCols as c}
                {#if !c.sort}
                  <div class="ts-th ts-al-{c.align}">{c.label}</div>
                {:else}
                  <button
                    class="ts-th ts-al-{c.align}"
                    class:is-sorted={sortKey === c.k}
                    onclick={() => onSortSessions(c.k as SessionSortKey)}
                  >
                    {c.label}
                    {#if sortKey === c.k}
                      <span class="ts-sortarrow">{sortDir === "asc" ? "▴" : "▾"}</span>
                    {/if}
                  </button>
                {/if}
              {/each}
            </div>
            <div class="ts-tbody">
              {#if filteredSessions.length === 0}
                <div class="ts-empty">No sessions match these filters.</div>
              {/if}
              {#each filteredSessions as s (s.session_id)}
                {@const pct = Math.round((s.context_tokens / s.context_limit) * 100)}
                {@const tier = contextTier(pct)}
                {@const open = openId === s.session_id}
                {@const isHidden = hidden.has(s.session_id)}
                {@const subTok = totalTokens(s.subagent_tokens)}
                <div
                  class="ts-rowwrap"
                  class:is-open={open}
                  class:is-hidden-row={isHidden}
                >
                  <div
                    class="ts-tr ts-sess-grid ts-sess-row"
                    role="button"
                    tabindex="0"
                    onclick={() =>
                      (openId = open ? null : s.session_id)}
                    onkeydown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        openId = open ? null : s.session_id;
                      }
                    }}
                  >
                    <div class="ts-td"><StatusDot status={s.status} /></div>
                    <div class="ts-td ts-sess-name">
                      <span class="ts-row-caret">
                        <Icon
                          name={open ? "chevronDown" : "chevronRight"}
                          size={13}
                        />
                      </span>
                      <span class={s.name ? "" : "ts-mono ts-text2"}>
                        {s.name ?? s.session_id.slice(0, 8)}
                      </span>
                      {#if !s.name}
                        <span
                          class="ts-sess-copy"
                          onclick={(e) => e.stopPropagation()}
                          role="presentation"
                        >
                          <CopyButton text={s.session_id} title="Copy UID" />
                        </span>
                      {/if}
                    </div>
                    <div class="ts-td ts-mono ts-text2">{repoName(s.cwd)}</div>
                    <div class="ts-td">
                      <span class="ts-modeltag">{fmtModel(s.model)}</span>
                    </div>
                    <div class="ts-td ts-ctx-cell">
                      <ContextBar {pct} height={4} />
                      <span
                        class="ts-tnum ts-ctx-pct"
                        style="color:var(--ts-tier-{tier});">{pct}%</span
                      >
                    </div>
                    <div
                      class="ts-td ts-al-right ts-tnum"
                      class:ts-cost-strong={isSubs}
                      class:ts-text2={!isSubs}
                    >
                      {fmtTokensShort(totalTokens(s.tokens))}
                    </div>
                    <div
                      class="ts-td ts-al-right ts-tnum"
                      class:ts-cost-strong={!isSubs}
                      class:ts-text3={isSubs}
                    >
                      {fmtUSD(s.cost_usd)}
                    </div>
                    <div class="ts-td ts-al-right ts-tnum ts-text3">
                      {s.subagent_count || "—"}
                    </div>
                    <div class="ts-td ts-al-right ts-tnum ts-text3">
                      {fmtRelTime(s.updated_at_ms, now)}
                    </div>
                    <div
                      class="ts-td ts-al-right ts-row-actions"
                      onclick={(e) => e.stopPropagation()}
                      role="presentation"
                    >
                      {#if showHidden}
                        <button
                          class="ts-iconbtn"
                          title={isHidden ? "Unhide" : "Hide"}
                          onclick={() => toggleHide(s)}
                        >
                          <Icon name={isHidden ? "eye" : "eyeOff"} size={14} />
                        </button>
                      {:else}
                        <button
                          class="ts-iconbtn"
                          title="Hide"
                          onclick={() => toggleHide(s)}
                        >
                          <Icon name="eyeOff" size={14} />
                        </button>
                      {/if}
                      <button
                        class="ts-iconbtn is-danger"
                        title={s.status === "inactive"
                          ? "Delete"
                          : "Only inactive sessions can be deleted"}
                        disabled={s.status !== "inactive"}
                        onclick={() => (confirm = s)}
                      >
                        <Icon name="trash" size={14} />
                      </button>
                    </div>
                  </div>
                  {#if open}
                    <div class="ts-rowdetail">
                      <div class="ts-rd-grid">
                        <div class="ts-tokcol">
                          <div class="ts-tokcol-title">Session tokens</div>
                          <div class="ts-tokrow"><span>input</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.input)}</span></div>
                          <div class="ts-tokrow"><span>output</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.output)}</span></div>
                          <div class="ts-tokrow"><span>cache write</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.cache_creation)}</span></div>
                          <div class="ts-tokrow"><span>cache read</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.cache_read)}</span></div>
                        </div>
                        <div class="ts-tokcol" class:is-muted={subTok === 0}>
                          <div class="ts-tokcol-title">
                            Subagent tokens ({s.subagent_count})
                          </div>
                          <div class="ts-tokrow"><span>input</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.input)}</span></div>
                          <div class="ts-tokrow"><span>output</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.output)}</span></div>
                          <div class="ts-tokrow"><span>cache write</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.cache_creation)}</span></div>
                          <div class="ts-tokrow"><span>cache read</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.cache_read)}</span></div>
                        </div>
                        <div class="ts-rd-meta">
                          <div class="ts-drow"><span class="ts-drow-k">Context</span><span class="ts-drow-v ts-tnum" style="color:var(--ts-tier-{tier});">{fmtTokensFull(s.context_tokens)} / {fmtTokensShort(s.context_limit)} · {pct}%</span></div>
                          <div class="ts-drow">
                            <span class="ts-drow-k">Model</span>
                            <span class="ts-drow-v-wrap">
                              <span class="ts-drow-v ts-mono">{s.model ?? "—"}</span>
                              {#if s.model}<CopyButton text={s.model} title="Copy model" />{/if}
                            </span>
                          </div>
                          <div class="ts-drow"><span class="ts-drow-k">Cost</span><span class="ts-drow-v ts-tnum">{fmtUSD(s.cost_usd)}</span></div>
                          <div class="ts-drow"><span class="ts-drow-k">Updated</span><span class="ts-drow-v ts-tnum">{fmtRelTime(s.updated_at_ms, now)}</span></div>
                          <div class="ts-drow"><span class="ts-drow-k">PID</span><span class="ts-drow-v ts-mono ts-tnum">{s.pid ?? "— not running"}</span></div>
                          <div class="ts-drow">
                            <span class="ts-drow-k">Path</span>
                            <span class="ts-drow-v-wrap">
                              <span class="ts-drow-v ts-mono is-ellipsis" title={s.cwd ?? ""}>{s.cwd ?? "—"}</span>
                              {#if s.cwd}<CopyButton text={s.cwd} title="Copy path" />{/if}
                            </span>
                          </div>
                          <div class="ts-drow">
                            <span class="ts-drow-k">UID</span>
                            <span class="ts-drow-v-wrap">
                              <span class="ts-drow-v ts-mono is-ellipsis" title={s.session_id}>{s.session_id}</span>
                              <CopyButton text={s.session_id} title="Copy UID" />
                            </span>
                          </div>
                        </div>
                      </div>
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        {:else if view === "repos"}
          <div class="ts-dash-h1row">
            <h1 class="ts-dash-h1">Repositories</h1>
            <span class="ts-dash-sub ts-tnum">
              {#if isSubs}
                {repos.length} repos · {fmtTokensShort(grandTokens)} tokens
                · <span class="ts-text3">~{fmtUSD(grandCost)} equiv</span>
              {:else}
                {repos.length} repos · {fmtUSD(grandCost)} total this week
              {/if}
            </span>
          </div>

          <div class="ts-table ts-repotable">
            <div class="ts-tr ts-tr-head">
              {#each repoCols as c}
                <button
                  class="ts-th ts-al-{c.align}"
                  class:is-sorted={repoSortKey === c.k}
                  onclick={() => onSortRepos(c.k)}
                >
                  {c.label}
                  {#if repoSortKey === c.k}
                    <span class="ts-sortarrow">{repoSortDir === "asc" ? "▴" : "▾"}</span>
                  {/if}
                </button>
              {/each}
            </div>
            <div class="ts-tbody">
              {#if sortedRepos.length === 0}
                <div class="ts-empty">No repositories yet.</div>
              {/if}
              {#each sortedRepos as r}
                <div class="ts-tr ts-repo-row">
                  <div class="ts-td ts-repo-name">
                    <span class="ts-repo-ico"><Icon name="folder" size={14} /></span>
                    <span class="ts-mono">{r.repo}</span>
                  </div>
                  <div class="ts-td ts-al-right ts-tnum">{r.session_count}</div>
                  <div class="ts-td ts-al-right ts-tnum">
                    {#if r.live_count > 0}
                      <span class="ts-live-pill"
                        ><span class="ts-live-pip"></span>{r.live_count}</span
                      >
                    {:else}
                      <span class="ts-text3">0</span>
                    {/if}
                  </div>
                  <div
                    class="ts-td ts-al-right ts-tnum"
                    class:ts-text2={!isSubs}
                    class:ts-cost-strong={isSubs}
                  >
                    {fmtTokensShort(r.total_tokens)}
                  </div>
                  <div class="ts-td ts-al-right ts-cost-cell">
                    {#if isSubs}
                      <div class="ts-cost-bar">
                        <div
                          class="ts-cost-fill"
                          style="width:{(r.total_tokens / maxRepoTokens) * 100}%;"
                        ></div>
                      </div>
                      <span class="ts-tnum ts-cost-val-subs">
                        {fmtTokensShort(r.total_tokens)}
                        <span class="ts-cost-shadow ts-tnum"
                          >~{fmtUSD(r.total_cost_usd)}</span
                        >
                      </span>
                    {:else}
                      <div class="ts-cost-bar">
                        <div
                          class="ts-cost-fill"
                          style="width:{(r.total_cost_usd / maxRepoCost) * 100}%;"
                        ></div>
                      </div>
                      <span class="ts-tnum ts-cost-val">
                        {fmtUSD(r.total_cost_usd)}
                      </span>
                    {/if}
                  </div>
                  <div class="ts-td">
                    {#if r.top_model}
                      <span class="ts-modeltag">{fmtModel(r.top_model)}</span>
                    {:else}
                      <span class="ts-text3">—</span>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    </main>
  </div>

  {#if confirm}
    {@const s = confirm}
    {@const name = s.name || s.session_id.slice(0, 8)}
    <div
      class="ts-modal-scrim"
      onclick={() => (confirm = null)}
      role="presentation"
    >
      <div
        class="ts-modal"
        onclick={(e) => e.stopPropagation()}
        onkeydown={(e) => e.key === "Escape" && (confirm = null)}
        role="dialog"
        aria-modal="true"
        tabindex="-1"
      >
        <div class="ts-modal-icon"><Icon name="trash" size={18} /></div>
        <div class="ts-modal-title">Delete this session?</div>
        <div class="ts-modal-body">
          <span class="ts-modal-name">{name}</span> and its logs will be
          permanently removed from
          <span class="ts-mono"> ~/.claude/</span>. This can't be undone.
        </div>
        <div class="ts-modal-actions">
          <button class="ts-btn ts-btn-ghost" onclick={() => (confirm = null)}
            >Cancel</button
          >
          <button class="ts-btn ts-btn-danger" onclick={() => deleteSession(s)}
            >Delete session</button
          >
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .dash-win {
    position: relative;
    width: 100vw;
    height: 100vh;
    background: var(--ts-bg-dashboard);
    color: var(--ts-text-1);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .dash-body {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  /* sidebar */
  .dash-sidebar {
    width: 210px;
    flex-shrink: 0;
    background: var(--ts-bg-sidebar);
    border-right: 1px solid var(--ts-border);
    padding: 12px 10px;
    display: flex;
    flex-direction: column;
  }
  .dash-sb-section {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.6px;
    color: var(--ts-text-3);
    padding: 6px 8px 8px;
  }
  .dash-navitem {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border-radius: 7px;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: var(--ts-font);
    font-size: 13.5px;
    font-weight: 500;
    color: var(--ts-text-2);
    transition: 0.12s;
    text-align: left;
  }
  .dash-navitem:hover {
    background: var(--ts-surface);
    color: var(--ts-text-1);
  }
  .dash-navitem.is-on {
    background: var(--ts-accent);
    color: #fff;
    font-weight: 550;
  }
  .dash-nav-count {
    margin-left: auto;
    font-size: 11.5px;
    color: var(--ts-text-3);
    background: var(--ts-surface);
    padding: 1px 7px;
    border-radius: 10px;
  }
  .dash-navitem.is-on .dash-nav-count {
    background: rgba(255, 255, 255, 0.22);
    color: #fff;
  }
  .dash-sb-spacer {
    flex: 1;
  }
  .dash-sb-foot {
    padding: 8px;
    border-top: 1px solid var(--ts-border);
  }
  .dash-sb-live {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 11.5px;
    color: var(--ts-text-2);
  }
  .dash-livepip {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ts-st-active);
  }
  .dash-sb-path {
    font-size: 11px;
    color: var(--ts-text-3);
    margin-top: 6px;
  }

  /* main */
  .dash-main {
    flex: 1;
    min-width: 0;
    background: var(--ts-bg-content);
    overflow: hidden;
    display: flex;
  }
  .ts-dashbody {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 22px 26px;
    overflow: hidden;
    position: relative;
  }
  .ts-dash-h1row {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin-bottom: 18px;
    flex-shrink: 0;
  }
  .ts-dash-h1 {
    font-size: 21px;
    font-weight: 650;
    letter-spacing: -0.3px;
    margin: 0;
  }
  .ts-dash-sub {
    font-size: 12.5px;
    color: var(--ts-text-3);
  }

  /* toolbar */
  .ts-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
    flex-shrink: 0;
  }
  .ts-searchbox {
    display: flex;
    align-items: center;
    gap: 7px;
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border-2);
    border-radius: 8px;
    padding: 0 10px;
    height: 32px;
    width: 250px;
  }
  .ts-searchbox:focus-within {
    border-color: var(--ts-accent);
    box-shadow: 0 0 0 3px var(--ts-accent-weak);
  }
  .ts-search-ico {
    color: var(--ts-text-3);
    display: inline-flex;
  }
  .ts-search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    font-family: var(--ts-font);
    font-size: 13px;
    color: var(--ts-text-1);
  }
  .ts-search-input::placeholder {
    color: var(--ts-text-3);
  }
  .ts-search-clear {
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--ts-text-3);
    display: flex;
    padding: 2px;
    border-radius: 4px;
  }
  .ts-search-clear:hover {
    color: var(--ts-text-1);
    background: var(--ts-surface-hi);
  }
  .ts-segmented {
    display: flex;
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border-2);
    border-radius: 8px;
    padding: 2px;
    gap: 1px;
  }
  .ts-seg {
    display: flex;
    align-items: center;
    gap: 5px;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: var(--ts-font);
    font-size: 12px;
    color: var(--ts-text-2);
    padding: 5px 10px;
    border-radius: 6px;
    text-transform: capitalize;
    transition: 0.1s;
  }
  .ts-seg:hover {
    color: var(--ts-text-1);
  }
  .ts-seg.is-on {
    background: var(--ts-surface-hi);
    color: var(--ts-text-1);
    font-weight: 550;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
  .ts-select {
    font-family: var(--ts-font);
    font-size: 12.5px;
    color: var(--ts-text-1);
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border-2);
    border-radius: 8px;
    padding: 0 28px 0 10px;
    height: 32px;
    cursor: pointer;
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23888' stroke-width='2.5'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 9px center;
  }
  .ts-ghostbtn {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border-2);
    border-radius: 8px;
    height: 32px;
    padding: 0 12px;
    cursor: pointer;
    font-family: var(--ts-font);
    font-size: 12.5px;
    color: var(--ts-text-2);
    transition: 0.12s;
  }
  .ts-ghostbtn:hover {
    color: var(--ts-text-1);
    border-color: var(--ts-text-3);
  }
  .ts-ghostbtn.is-on {
    color: var(--ts-accent);
    border-color: var(--ts-accent-line);
    background: var(--ts-accent-weak);
  }

  /* table */
  .ts-table {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border: 1px solid var(--ts-border);
    border-radius: 10px;
    overflow: hidden;
    background: var(--ts-bg-content);
  }
  .ts-tr-head {
    background: var(--ts-surface-2);
    border-bottom: 1px solid var(--ts-border);
    flex-shrink: 0;
  }
  .ts-th {
    font-family: var(--ts-font);
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.3px;
    color: var(--ts-text-3);
    text-transform: uppercase;
    background: transparent;
    border: none;
    cursor: pointer;
    padding: 9px 10px;
    display: flex;
    align-items: center;
    gap: 3px;
  }
  .ts-th:hover {
    color: var(--ts-text-1);
  }
  .ts-th.is-sorted {
    color: var(--ts-text-1);
  }
  .ts-al-right {
    justify-content: flex-end;
    text-align: right;
  }
  .ts-sortarrow {
    font-size: 9px;
  }
  .ts-tbody {
    flex: 1;
    overflow-y: auto;
  }
  .ts-tbody::-webkit-scrollbar {
    width: 10px;
  }
  .ts-tbody::-webkit-scrollbar-thumb {
    background: var(--ts-border-2);
    border-radius: 6px;
    border: 2px solid var(--ts-bg-content);
  }

  /* repos table grid */
  .ts-repotable .ts-tr {
    display: grid;
    grid-template-columns: 1.6fr 0.9fr 0.7fr 1fr 1.4fr 1.2fr;
    align-items: center;
  }
  .ts-repo-row {
    padding: 0;
    border-bottom: 1px solid var(--ts-border);
  }
  .ts-repo-row:hover {
    background: var(--ts-surface-2);
  }
  .ts-td {
    padding: 11px 10px;
    font-size: 13px;
  }
  .ts-repo-name {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ts-repo-ico {
    color: var(--ts-text-3);
    display: inline-flex;
  }
  .ts-live-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--ts-st-active);
    font-weight: 600;
  }
  .ts-live-pip {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ts-st-active);
  }
  .ts-cost-cell {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: flex-end;
  }
  .ts-cost-bar {
    width: 70px;
    height: 6px;
    border-radius: 3px;
    background: var(--ts-surface-hi);
    overflow: hidden;
  }
  .ts-cost-fill {
    height: 100%;
    border-radius: 3px;
    background: var(--ts-accent);
  }
  .ts-cost-val {
    font-weight: 600;
    min-width: 52px;
    text-align: right;
  }
  .ts-cost-val-subs {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 1px;
    min-width: 70px;
  }
  .ts-cost-val-subs {
    font-weight: 600;
  }
  .ts-cost-shadow {
    font-size: 10.5px;
    color: var(--ts-text-3);
    font-weight: 500;
  }

  /* sessions table grid */
  .ts-sess-grid {
    display: grid;
    grid-template-columns: 26px 1.7fr 1fr 1fr 1.3fr 0.9fr 0.8fr 0.5fr 1fr 76px;
    align-items: center;
  }
  .ts-sess-row {
    cursor: pointer;
  }
  .ts-rowwrap {
    border-bottom: 1px solid var(--ts-border);
  }
  .ts-rowwrap:hover {
    background: var(--ts-surface-2);
  }
  .ts-rowwrap.is-open {
    background: var(--ts-surface-2);
  }
  .ts-rowwrap.is-hidden-row {
    opacity: 0.5;
  }
  .ts-sess-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
  }
  .ts-sess-name > span:last-child {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ts-row-caret {
    color: var(--ts-text-3);
    flex-shrink: 0;
    display: inline-flex;
  }
  .ts-sess-copy {
    display: inline-flex;
    opacity: 0;
    transition: opacity 0.12s;
  }
  .ts-rowwrap:hover .ts-sess-copy,
  .ts-rowwrap.is-open .ts-sess-copy {
    opacity: 1;
  }
  .ts-ctx-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ts-ctx-pct {
    font-size: 11px;
    font-weight: 600;
    width: 30px;
    text-align: right;
    flex-shrink: 0;
  }
  .ts-cost-strong {
    font-weight: 600;
  }
  .ts-row-actions {
    display: flex;
    gap: 2px;
    justify-content: flex-end;
    opacity: 0;
    transition: opacity 0.12s;
  }
  .ts-rowwrap:hover .ts-row-actions,
  .ts-rowwrap.is-open .ts-row-actions {
    opacity: 1;
  }
  .ts-iconbtn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--ts-text-2);
    transition: 0.12s;
  }
  .ts-iconbtn:hover {
    background: var(--ts-surface-hi);
    color: var(--ts-text-1);
  }
  .ts-iconbtn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .ts-iconbtn.is-danger:not(:disabled):hover {
    background: rgba(239, 68, 68, 0.14);
    color: var(--ts-tier-critical);
  }
  .ts-rowdetail {
    padding: 4px 16px 16px 38px;
    background: var(--ts-surface-2);
    border-top: 1px solid var(--ts-border);
    animation: ts-fadein 0.14s ease;
  }
  @keyframes ts-fadein {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  .ts-rd-grid {
    display: grid;
    grid-template-columns: 1fr 1fr 1.3fr;
    gap: 10px 16px;
  }
  .ts-tokcol-title {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.4px;
    color: var(--ts-text-3);
    margin-bottom: 5px;
  }
  .ts-tokcol.is-muted {
    opacity: 0.4;
  }
  .ts-tokrow {
    display: flex;
    justify-content: space-between;
    font-size: 11.5px;
    color: var(--ts-text-2);
    padding: 1.5px 0;
  }
  .ts-tokrow .ts-mono {
    color: var(--ts-text-1);
  }
  .ts-rd-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ts-drow {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 11.5px;
    padding: 2px 0;
  }
  .ts-drow-k {
    color: var(--ts-text-3);
    flex-shrink: 0;
  }
  .ts-drow-v {
    color: var(--ts-text-1);
    text-align: right;
  }
  .ts-drow-v.is-ellipsis {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 230px;
    direction: rtl;
  }
  .ts-drow-v-wrap {
    display: inline-flex;
    align-items: center;
    min-width: 0;
    flex: 0 1 auto;
  }
  .ts-empty {
    padding: 40px;
    text-align: center;
    color: var(--ts-text-3);
    font-size: 13px;
  }
  .ts-err {
    color: var(--ts-tier-critical);
  }
  .ts-text2 {
    color: var(--ts-text-2);
  }
  .ts-text3 {
    color: var(--ts-text-3);
  }

  .ts-modeltag {
    font-family: var(--ts-mono);
    font-size: 10.5px;
    padding: 2px 6px;
    border-radius: 5px;
    background: var(--ts-surface-hi);
    color: var(--ts-text-2);
    white-space: nowrap;
  }

  /* modal */
  .ts-modal-scrim {
    position: absolute;
    inset: 0;
    background: var(--ts-scrim);
    backdrop-filter: blur(2px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
    animation: ts-fadein 0.12s;
  }
  .ts-modal {
    width: 360px;
    background: var(--ts-bg-popover);
    border-radius: 12px;
    padding: 22px;
    box-shadow: var(--ts-win-shadow);
    text-align: center;
    animation: ts-modal-pop 0.16s cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes ts-modal-pop {
    from {
      opacity: 0;
      transform: translateY(-6px) scale(0.985);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .ts-modal-icon {
    width: 42px;
    height: 42px;
    border-radius: 50%;
    background: rgba(239, 68, 68, 0.14);
    color: var(--ts-tier-critical);
    display: flex;
    align-items: center;
    justify-content: center;
    margin: 0 auto 14px;
  }
  .ts-modal-title {
    font-size: 16px;
    font-weight: 650;
    margin-bottom: 8px;
  }
  .ts-modal-body {
    font-size: 13px;
    color: var(--ts-text-2);
    line-height: 1.55;
    margin-bottom: 20px;
  }
  .ts-modal-name {
    font-weight: 600;
    color: var(--ts-text-1);
  }
  .ts-modal-actions {
    display: flex;
    gap: 10px;
  }
  .ts-btn {
    flex: 1;
    height: 38px;
    border-radius: 8px;
    border: none;
    cursor: pointer;
    font-family: var(--ts-font);
    font-size: 13px;
    font-weight: 600;
    transition: 0.12s;
  }
  .ts-btn-ghost {
    background: var(--ts-surface);
    color: var(--ts-text-1);
    border: 1px solid var(--ts-border-2);
  }
  .ts-btn-ghost:hover {
    background: var(--ts-surface-hi);
  }
  .ts-btn-danger {
    background: var(--ts-tier-critical);
    color: #fff;
  }
  .ts-btn-danger:hover {
    filter: brightness(1.08);
  }
</style>
