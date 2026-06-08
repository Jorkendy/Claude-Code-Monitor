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
    fmtClock,
    fmtDuration,
    fmtModel,
    contextTier,
    repoName,
    synthBurnSeries,
    type Tokens,
  } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import StatusDot from "$lib/components/StatusDot.svelte";
  import ContextBar from "$lib/components/ContextBar.svelte";
  import Sparkline from "$lib/components/Sparkline.svelte";
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

  type Plan = "api" | "pro" | "max-5x" | "max-20x";
  type Theme = "system" | "light" | "dark";
  type Settings = {
    budget_window_usd: number;
    plan: Plan;
    rate_limit_warn_pct: number;
    context_warn_pct: number;
    custom_quota: number | null;
    theme: Theme;
    first_run: boolean;
  };
  type Tab = "sessions" | "blocks" | "settings";

  // Community-estimated message quota per 5h window — mirrors backend.
  const QUOTA: Record<Plan, number | null> = {
    api: null,
    pro: 45,
    "max-5x": 225,
    "max-20x": 900,
  };
  const PLAN_LABEL: Record<Plan, string> = {
    api: "API",
    pro: "Pro",
    "max-5x": "Max 5×",
    "max-20x": "Max 20×",
  };

  let tab: Tab = $state("sessions");
  let sessions: Session[] = $state([]);
  let blocks: BlockView[] = $state([]);
  let settings: Settings = $state({
    budget_window_usd: 0,
    plan: "api",
    rate_limit_warn_pct: 90,
    context_warn_pct: 90,
    custom_quota: null,
    theme: "system",
    first_run: true,
  });
  let loading = $state(true);
  let error: string | null = $state(null);
  let now = $state(Date.now());
  let expandedId: string | null = $state(null);
  let hidden: Set<string> = $state(new Set());
  let lastLoadedAt: number = $state(Date.now());

  async function load(showSpinner = false) {
    try {
      if (showSpinner) loading = true;
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
      lastLoadedAt = Date.now();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function saveSettings() {
    try {
      await invoke("set_settings", { settings });
    } catch (e) {
      error = String(e);
    }
  }

  async function openDashboard() {
    await invoke("open_dashboard");
  }

  function toggleExpand(id: string) {
    expandedId = expandedId === id ? null : id;
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

  // --- derived ----------------------------------------------------------
  const shownSessions = $derived(sessions.filter((s) => !hidden.has(s.session_id)));
  const liveSessions = $derived(
    shownSessions.filter((s) => s.status === "active" || s.status === "idle"),
  );
  const inactiveHidden = $derived(shownSessions.length - liveSessions.length);

  // Sort: active first, then by context % desc (danger-first).
  const sortedSessions = $derived(
    [...liveSessions].sort((a, b) => {
      const rank = (s: Session) => (s.status === "active" ? 0 : 1);
      if (rank(a) !== rank(b)) return rank(a) - rank(b);
      return (
        b.context_tokens / b.context_limit - a.context_tokens / a.context_limit
      );
    }),
  );

  // Disambiguate duplicate names with a short UID suffix.
  const dupNames = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const s of sortedSessions) {
      if (s.name) counts.set(s.name, (counts.get(s.name) ?? 0) + 1);
    }
    const out = new Set<string>();
    for (const [n, c] of counts) if (c > 1) out.add(n);
    return out;
  });

  function displayName(s: Session): string {
    if (!s.name) return s.session_id.slice(0, 8);
    if (dupNames.has(s.name)) return `${s.name} #${s.session_id.slice(0, 8)}`;
    return s.name;
  }

  const activeBlock = $derived(blocks.find((b) => b.is_active));
  const recentBlocks = $derived(
    blocks
      .filter((b) => !b.is_active && !b.is_gap)
      .sort((a, b) => b.start_ms - a.start_ms)
      .slice(0, 5),
  );

  // Hero derivations.
  const heroBlock = $derived(activeBlock);
  const heroBurnSeries = $derived(synthBurnSeries(heroBlock?.burn_usd_per_hr ?? 0));
  const heroTimeProgress = $derived.by(() => {
    if (!heroBlock) return 0;
    const elapsed = now - heroBlock.start_ms;
    const total = heroBlock.end_ms - heroBlock.start_ms;
    return Math.min(100, Math.max(0, (elapsed / total) * 100));
  });
  const heroRemaining = $derived(heroBlock ? heroBlock.end_ms - now : 0);

  // Subscription-mode derivations. Custom quota (when set) overrides the
  // community estimate, matching backend `effective_quota`.
  const baseQuota = $derived(QUOTA[settings.plan]);
  const quota = $derived(
    baseQuota != null && settings.custom_quota && settings.custom_quota > 0
      ? settings.custom_quota
      : baseQuota,
  );
  const isSubs = $derived(quota != null);
  const quotaPct = $derived.by(() => {
    if (!heroBlock || quota == null || quota === 0) return 0;
    return Math.min(999, (heroBlock.message_count / quota) * 100);
  });
  const quotaTier = $derived(contextTier(quotaPct));
  // Linear extrapolation across the 5h window. Block.message_count grows
  // monotonically; if we've used N msgs in T elapsed minutes of a 5h window,
  // straight-line projects to N * 300/T by reset.
  const projectedMsgs = $derived.by(() => {
    if (!heroBlock) return 0;
    const elapsed = now - heroBlock.start_ms;
    if (elapsed < 60_000) return heroBlock.message_count;
    const total = heroBlock.end_ms - heroBlock.start_ms;
    return Math.round((heroBlock.message_count * total) / elapsed);
  });
  const msgPerHr = $derived.by(() => {
    if (!heroBlock) return 0;
    const elapsedHr = Math.max(1 / 60, (now - heroBlock.start_ms) / 3_600_000);
    return heroBlock.message_count / elapsedHr;
  });
  // Drives the bottom progress bar. Time elapsed in API mode (so user sees
  // window emptying); quota % in subscription mode (the real concern).
  const heroBarPct = $derived(isSubs ? Math.min(100, quotaPct) : heroTimeProgress);
  const heroBarColor = $derived(
    isSubs ? `var(--ts-tier-${quotaTier})` : "var(--ts-accent)",
  );

  // --- theme ---
  // `system` clears data-theme so tokens.css's prefers-color-scheme rule
  // wins; explicit values pin the override.
  $effect(() => {
    const root = document.documentElement;
    if (settings.theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", settings.theme);
  });

  // --- first-run modal ---
  let showFirstRun = $state(false);
  $effect(() => {
    if (!loading && settings.first_run) showFirstRun = true;
  });
  async function pickFirstRunPlan(p: Plan) {
    settings.plan = p;
    settings.first_run = false;
    showFirstRun = false;
    await saveSettings();
  }

  // --- keyboard shortcuts ---
  // Skip when an input/textarea is focused so users can still type numbers
  // into the Settings inputs.
  function handleKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
    if (showFirstRun) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    switch (e.key) {
      case "1": tab = "sessions"; break;
      case "2": tab = "blocks"; break;
      case "3": tab = "settings"; break;
      case "r": case "R": load(false); break;
      case "d": case "D": openDashboard(); break;
      case "Escape": {
        if (expandedId) { expandedId = null; break; }
        // No window.close in Tauri webview; emulate dismiss by blurring.
        (document.activeElement as HTMLElement | null)?.blur();
        break;
      }
      default: return;
    }
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={handleKey} />

<div class="ts-popover" data-popover-root>
  {#if showFirstRun}
    <div class="ts-firstrun-scrim" role="presentation">
      <div
        class="ts-firstrun"
        role="dialog"
        aria-modal="true"
        aria-labelledby="firstrun-title"
      >
        <div class="ts-firstrun-title" id="firstrun-title">
          Welcome to Tokenscope
        </div>
        <div class="ts-firstrun-sub">
          Pick how you pay so the numbers mean what you expect.
        </div>
        <div class="ts-firstrun-plans">
          {#each ["api", "pro", "max-5x", "max-20x"] as const as p}
            <button
              class="ts-firstrun-plan"
              onclick={() => pickFirstRunPlan(p)}
            >
              <div class="ts-firstrun-plan-name">{PLAN_LABEL[p]}</div>
              <div class="ts-firstrun-plan-desc">
                {#if p === "api"}
                  Per-token cost via API
                {:else}
                  ~{QUOTA[p]} msgs / 5h window
                {/if}
              </div>
            </button>
          {/each}
        </div>
        <div class="ts-firstrun-foot">
          You can change this any time in Settings.
        </div>
      </div>
    </div>
  {/if}
  {#if heroBlock}
    {@const burnHot = heroBlock.burn_usd_per_hr >= 0.5}
    {@const rateHot = isSubs ? quotaPct >= 50 : burnHot}
    <div class="ts-hero">
      <div class="ts-hero-top">
        <div class="ts-hero-block">
          {#if isSubs}
            <div class="ts-hero-label">5H WINDOW MESSAGES</div>
            <div class="ts-hero-cost ts-tnum">
              {heroBlock.message_count}<span class="ts-hero-quota-sep"
                >/{quota}</span
              >
            </div>
            <div class="ts-hero-proj ts-tnum">
              <span style="color:var(--ts-tier-{quotaTier});"
                >{quotaPct.toFixed(0)}%</span
              >
              · est. {projectedMsgs} by reset
            </div>
          {:else}
            <div class="ts-hero-label">CURRENT BLOCK</div>
            <div class="ts-hero-cost ts-tnum">{fmtUSD(heroBlock.cost_usd)}</div>
            <div class="ts-hero-proj ts-tnum">
              est. {fmtUSD(heroBlock.projected_block_usd)}
            </div>
          {/if}
        </div>
        <div class="ts-hero-burn">
          <div class="ts-hero-label ts-hero-label-r">
            {isSubs ? "PACE" : "BURN RATE"}
          </div>
          <div class="ts-hero-sparkrow">
            <Sparkline
              data={heroBurnSeries}
              width={120}
              height={30}
              stroke={rateHot ? "var(--ts-accent)" : "var(--ts-text-3)"}
            />
          </div>
          <div
            class="ts-hero-burnval-sm ts-tnum"
            style="color:{rateHot ? 'var(--ts-burn-hot)' : 'var(--ts-text-2)'};"
          >
            {#if isSubs}
              {msgPerHr.toFixed(0)}<span class="ts-hero-unit">msg/hr</span>
            {:else}
              {fmtUSD(heroBlock.burn_usd_per_hr)}<span class="ts-hero-unit"
                >/hr</span
              >
            {/if}
          </div>
        </div>
      </div>

      <div class="ts-hero-reset">
        <div class="ts-hero-resetbar">
          <div
            class="ts-hero-resetfill"
            style="width:{heroBarPct}%;background:{heroBarColor};"
          ></div>
        </div>
        <div class="ts-hero-resetmeta">
          <span>{liveSessions.length} live</span>
          <span class="ts-tnum">resets in {fmtDuration(heroRemaining)}</span>
        </div>
      </div>
    </div>
  {:else}
    <div class="ts-hero ts-hero-empty">
      <div class="ts-hero-label">
        {isSubs ? "5H WINDOW MESSAGES" : "CURRENT BLOCK"}
      </div>
      <div class="ts-hero-cost ts-tnum">—</div>
      <div class="ts-hero-proj">No active block.</div>
    </div>
  {/if}

  <nav class="ts-pop-tabs">
    <button
      class="ts-tab"
      class:is-on={tab === "sessions"}
      onclick={() => (tab = "sessions")}
    >Sessions</button>
    <button
      class="ts-tab"
      class:is-on={tab === "blocks"}
      onclick={() => (tab = "blocks")}
    >Blocks</button>
    <button
      class="ts-tab"
      class:is-on={tab === "settings"}
      onclick={() => (tab = "settings")}
    >Settings</button>
  </nav>

  <div class="ts-pop-scroll">
    {#if loading}
      <div class="ts-tabbody ts-empty-state">Loading…</div>
    {:else if error}
      <div class="ts-tabbody ts-empty-state ts-err">{error}</div>
    {:else if tab === "sessions"}
      <div class="ts-tabbody ts-sessions">
        {#if sortedSessions.length === 0}
          <div class="ts-empty-state">No live sessions.</div>
        {:else}
          {#each sortedSessions as s (s.session_id)}
            {@const pct = Math.round((s.context_tokens / s.context_limit) * 100)}
            {@const tier = contextTier(pct)}
            {@const open = expandedId === s.session_id}
            {@const subTok = totalTokens(s.subagent_tokens)}
            <div
              class="ts-card"
              class:is-open={open}
              onclick={() => toggleExpand(s.session_id)}
              role="button"
              tabindex="0"
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  toggleExpand(s.session_id);
                }
              }}
            >
              <div class="ts-card-head">
                <div class="ts-card-id">
                  <StatusDot status={s.status} />
                  <span class="ts-card-name" class:is-mono={!s.name}
                    >{displayName(s)}</span
                  >
                </div>
                {#if isSubs}
                  <div class="ts-card-cost ts-tnum">
                    {fmtTokensShort(totalTokens(s.tokens))}<span
                      class="ts-card-unit">tok</span
                    >
                  </div>
                {:else}
                  <div class="ts-card-cost ts-tnum">{fmtUSD(s.cost_usd)}</div>
                {/if}
              </div>

              <div class="ts-card-ctx">
                <ContextBar {pct} height={4} />
                <span
                  class="ts-card-ctxpct ts-tnum"
                  style="color:var(--ts-tier-{tier});">{pct}%</span
                >
              </div>

              <div class="ts-card-meta">
                <span class="ts-mono">{repoName(s.cwd)}</span>
                <span class="ts-meta-dot">·</span>
                <span>{fmtModel(s.model)}</span>
                {#if s.subagent_count > 0}
                  <span class="ts-meta-dot">·</span>
                  <span>{s.subagent_count} sub</span>
                {/if}
                <span class="ts-card-time">{fmtRelTime(s.updated_at_ms, now)}</span>
              </div>

              {#if open}
                <div
                  class="ts-card-detail"
                  onclick={(e) => e.stopPropagation()}
                  role="presentation"
                >
                  <div class="ts-detail-grid">
                    <div class="ts-tokcol">
                      <div class="ts-tokcol-title">Session</div>
                      <div class="ts-tokrow"><span>input</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.input)}</span></div>
                      <div class="ts-tokrow"><span>output</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.output)}</span></div>
                      <div class="ts-tokrow"><span>cache write</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.cache_creation)}</span></div>
                      <div class="ts-tokrow"><span>cache read</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.tokens.cache_read)}</span></div>
                    </div>
                    <div class="ts-tokcol" class:is-muted={subTok === 0}>
                      <div class="ts-tokcol-title">Subagents ({s.subagent_count})</div>
                      <div class="ts-tokrow"><span>input</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.input)}</span></div>
                      <div class="ts-tokrow"><span>output</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.output)}</span></div>
                      <div class="ts-tokrow"><span>cache write</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.cache_creation)}</span></div>
                      <div class="ts-tokrow"><span>cache read</span><span class="ts-tnum ts-mono">{fmtTokensShort(s.subagent_tokens.cache_read)}</span></div>
                    </div>
                  </div>
                  <div class="ts-detail-rows">
                    <div class="ts-drow"><span class="ts-drow-k">Context</span><span class="ts-drow-v ts-tnum" style="color:var(--ts-tier-{tier});">{fmtTokensFull(s.context_tokens)} / {fmtTokensShort(s.context_limit)}</span></div>
                    <div class="ts-drow">
                      <span class="ts-drow-k">Model</span>
                      <span class="ts-drow-v-wrap">
                        <span class="ts-drow-v ts-mono">{s.model ?? "—"}</span>
                        {#if s.model}<CopyButton text={s.model} title="Copy model" />{/if}
                      </span>
                    </div>
                    <div class="ts-drow"><span class="ts-drow-k">Cost</span><span class="ts-drow-v ts-tnum">{fmtUSD(s.cost_usd)}</span></div>
                    <div class="ts-drow"><span class="ts-drow-k">Updated</span><span class="ts-drow-v ts-tnum">{fmtRelTime(s.updated_at_ms, now)}</span></div>
                    <div class="ts-drow">
                      <span class="ts-drow-k">Path</span>
                      <span class="ts-drow-v-wrap">
                        <span class="ts-drow-v ts-mono is-ellipsis" title={s.cwd ?? ""}>{s.cwd ?? "—"}</span>
                        {#if s.cwd}<CopyButton text={s.cwd} title="Copy path" />{/if}
                      </span>
                    </div>
                    <div class="ts-drow"><span class="ts-drow-k">PID</span><span class="ts-drow-v ts-mono ts-tnum">{s.pid ?? "—"}</span></div>
                    <div class="ts-drow">
                      <span class="ts-drow-k">UID</span>
                      <span class="ts-drow-v-wrap">
                        <span class="ts-drow-v ts-mono is-ellipsis" title={s.session_id}>{s.session_id}</span>
                        <CopyButton text={s.session_id} title="Copy UID" />
                      </span>
                    </div>
                  </div>
                </div>
              {/if}
            </div>
          {/each}
          {#if inactiveHidden > 0}
            <div class="ts-sessions-foot">
              {inactiveHidden} inactive session{inactiveHidden !== 1 ? "s" : ""}
              hidden · open dashboard to manage
            </div>
          {/if}
        {/if}
      </div>
    {:else if tab === "blocks"}
      <div class="ts-tabbody">
        {#if activeBlock}
          {@const burnHot = activeBlock.burn_usd_per_hr >= 0.5}
          {@const elapsed = now - activeBlock.start_ms}
          {@const total = activeBlock.end_ms - activeBlock.start_ms}
          {@const timePct = Math.min(100, Math.max(0, (elapsed / total) * 100))}
          {@const remaining = activeBlock.end_ms - now}
          {@const barPct = isSubs ? Math.min(100, quotaPct) : timePct}
          {@const barColor = isSubs
            ? `var(--ts-tier-${quotaTier})`
            : "var(--ts-accent)"}
          <div class="ts-activeblock">
            <div class="ts-ab-head">
              <span class="ts-ab-live"
                ><span class="ts-ab-livedot"></span>ACTIVE BLOCK</span
              >
              <span class="ts-ab-window ts-tnum"
                >{fmtClock(activeBlock.start_ms)} – {fmtClock(activeBlock.end_ms)}</span
              >
            </div>
            {#if isSubs}
              <div class="ts-ab-costrow">
                <div class="ts-ab-block-primary">
                  <div class="ts-ab-cost ts-tnum">
                    {activeBlock.message_count}<span class="ts-ab-quota-sep"
                      >/{quota}</span
                    >
                  </div>
                  <div class="ts-ab-shadow ts-tnum">
                    ~{fmtUSD(activeBlock.cost_usd)} equiv
                  </div>
                </div>
                <div class="ts-ab-proj">
                  <div class="ts-ab-projval ts-tnum">{projectedMsgs}</div>
                  <div class="ts-ab-projlbl">msgs by reset</div>
                </div>
              </div>
            {:else}
              <div class="ts-ab-costrow">
                <div class="ts-ab-cost ts-tnum">{fmtUSD(activeBlock.cost_usd)}</div>
                <div class="ts-ab-proj">
                  <div class="ts-ab-projval ts-tnum">
                    {fmtUSD(activeBlock.projected_block_usd)}
                  </div>
                  <div class="ts-ab-projlbl">projected total</div>
                </div>
              </div>
            {/if}
            <div class="ts-ab-resetbar">
              <div
                class="ts-ab-resetfill"
                style="width:{barPct}%;background:{barColor};"
              ></div>
            </div>
            <div class="ts-ab-resetmeta ts-tnum">
              <span>
                {#if isSubs}
                  <span style="color:var(--ts-tier-{quotaTier});"
                    >{quotaPct.toFixed(0)}%</span
                  > quota
                {:else}
                  {Math.round(timePct)}% elapsed
                {/if}
              </span>
              <span>resets in {fmtDuration(remaining)}</span>
            </div>
            <div class="ts-ab-stats">
              <div class="ts-ab-stat">
                <div class="ts-ab-statlbl">{isSubs ? "PACE" : "BURN"}</div>
                <Sparkline
                  data={synthBurnSeries(activeBlock.burn_usd_per_hr)}
                  width={120}
                  height={28}
                  stroke={burnHot ? "var(--ts-accent)" : "var(--ts-text-3)"}
                />
                <div class="ts-ab-statval-sm ts-tnum">
                  {#if isSubs}
                    {msgPerHr.toFixed(0)} msg/hr
                  {:else}
                    {fmtUSD(activeBlock.burn_usd_per_hr)}/hr
                  {/if}
                </div>
              </div>
              <div class="ts-ab-stat">
                <div class="ts-ab-statlbl">{isSubs ? "TOKENS" : "MESSAGES"}</div>
                <div class="ts-ab-statval ts-tnum">
                  {#if isSubs}
                    {fmtTokensShort(totalTokens(activeBlock.tokens))}
                  {:else}
                    {activeBlock.message_count}
                  {/if}
                </div>
                <div class="ts-ab-statval-sm ts-tnum">
                  {#if isSubs}
                    {activeBlock.message_count} msgs
                  {:else}
                    {fmtTokensShort(totalTokens(activeBlock.tokens))} tok
                  {/if}
                </div>
              </div>
              <div class="ts-ab-stat">
                <div class="ts-ab-statlbl">MODELS</div>
                <div class="ts-ab-models">
                  {#each activeBlock.models as m}
                    <span class="ts-modeltag">{fmtModel(m)}</span>
                  {/each}
                </div>
              </div>
            </div>
          </div>
        {/if}

        {#if recentBlocks.length > 0}
          <div class="ts-section-label">RECENT BLOCKS</div>
          <div class="ts-blocktable">
            <div class="ts-bt-head">
              <span>window</span>
              <span class="ts-tnum ts-bt-r">msgs</span>
              <span class="ts-bt-r">tokens</span>
              <span class="ts-bt-r" class:ts-text3={isSubs}>cost</span>
            </div>
            {#each recentBlocks as b (b.start_ms)}
              <div class="ts-bt-row">
                <span class="ts-mono ts-tnum"
                  >{fmtClock(b.start_ms)}–{fmtClock(b.end_ms)}</span
                >
                <span
                  class="ts-tnum ts-bt-r"
                  class:ts-bt-cost={isSubs}>{b.message_count}</span
                >
                <span class="ts-tnum ts-bt-r ts-text2"
                  >{fmtTokensShort(totalTokens(b.tokens))}</span
                >
                <span
                  class="ts-tnum ts-bt-r"
                  class:ts-bt-cost={!isSubs}
                  class:ts-text3={isSubs}>{fmtUSD(b.cost_usd)}</span
                >
              </div>
            {/each}
          </div>
        {/if}

        {#if !activeBlock && recentBlocks.length === 0}
          <div class="ts-empty-state">No billing blocks yet.</div>
        {/if}
      </div>
    {:else if tab === "settings"}
      <div class="ts-tabbody ts-settings">
        <div class="ts-set-field">
          <div class="ts-set-label">Plan</div>
          <div class="ts-plan-seg">
            {#each ["api", "pro", "max-5x", "max-20x"] as const as p}
              <button
                class="ts-plan-opt"
                class:is-on={settings.plan === p}
                onclick={() => {
                  settings.plan = p;
                  saveSettings();
                }}
              >
                {PLAN_LABEL[p]}
              </button>
            {/each}
          </div>
          <div class="ts-set-hint">
            {#if settings.plan === "api"}
              Per-token cost via Anthropic API. Tokenscope shows USD and burn rate.
            {:else}
              Flat-fee subscription. Tokenscope shows messages and % of estimated
              {quota}-msg/5h quota.
              <span class="ts-mono">*</span> community estimate — actual limit may differ.
            {/if}
          </div>
        </div>

        <div class="ts-set-divider"></div>

        {#if isSubs}
          <div class="ts-set-field">
            <label class="ts-set-label" for="rate-input"
              >Rate-limit warning</label
            >
            <div class="ts-set-inputrow">
              <input
                id="rate-input"
                class="ts-set-input ts-tnum"
                type="number"
                min="0"
                max="100"
                step="5"
                bind:value={settings.rate_limit_warn_pct}
                onchange={saveSettings}
              />
              <span class="ts-set-suffix">% of quota</span>
            </div>
            <div class="ts-set-hint">
              Notify when messages used in current 5h window cross this %.
              <span class="ts-mono">0</span> disables alerts.
            </div>
          </div>

          <div class="ts-set-field">
            <label class="ts-set-label" for="quota-input"
              >Custom quota</label
            >
            <div class="ts-set-inputrow">
              <input
                id="quota-input"
                class="ts-set-input ts-tnum"
                type="number"
                min="0"
                step="5"
                placeholder={String(baseQuota ?? 0)}
                bind:value={settings.custom_quota}
                onchange={saveSettings}
              />
              <span class="ts-set-suffix">msgs / 5h</span>
            </div>
            <div class="ts-set-hint">
              Override the community estimate ({baseQuota} for {PLAN_LABEL[settings.plan]})
              with your measured limit. <span class="ts-mono">0</span> or empty falls back to default.
            </div>
          </div>
        {:else}
          <div class="ts-set-field">
            <label class="ts-set-label" for="budget-input"
              >Budget alert threshold</label
            >
            <div class="ts-set-inputrow">
              <span class="ts-set-prefix">$</span>
              <input
                id="budget-input"
                class="ts-set-input ts-tnum"
                type="number"
                min="0"
                step="1"
                bind:value={settings.budget_window_usd}
                onchange={saveSettings}
              />
              <span class="ts-set-suffix">/ 5h block</span>
            </div>
            <div class="ts-set-hint">
              Notify when projected block cost crosses this.
              <span class="ts-mono">0</span> disables alerts.
            </div>
          </div>
        {/if}

        <div class="ts-set-field">
          <label class="ts-set-label" for="ctx-warn-input"
            >Context warning</label
          >
          <div class="ts-set-inputrow">
            <input
              id="ctx-warn-input"
              class="ts-set-input ts-tnum"
              type="number"
              min="0"
              max="100"
              step="5"
              bind:value={settings.context_warn_pct}
              onchange={saveSettings}
            />
            <span class="ts-set-suffix">% of window</span>
          </div>
          <div class="ts-set-hint">
            Tray shows <span class="ts-mono">⚠ ctx NN%</span> when any active
            (or recently-used idle) session reaches this. <span class="ts-mono">0</span> disables.
          </div>
        </div>

        <div class="ts-set-divider"></div>

        <div class="ts-set-field">
          <div class="ts-set-label">Appearance</div>
          <div class="ts-plan-seg">
            {#each ["system", "light", "dark"] as const as t}
              <button
                class="ts-plan-opt"
                class:is-on={settings.theme === t}
                onclick={() => {
                  settings.theme = t;
                  saveSettings();
                }}
              >
                {t === "system" ? "System" : t === "light" ? "Light" : "Dark"}
              </button>
            {/each}
          </div>
          <div class="ts-set-hint">
            <span class="ts-mono">System</span> follows your macOS appearance.
          </div>
        </div>

        <div class="ts-set-divider"></div>

        <div class="ts-set-note">
          <Icon name="keyboard" size={14} />
          <div>
            <div class="ts-set-note-k">Shortcuts</div>
            <div class="ts-set-note-v">
              <span class="ts-mono">1</span>/<span class="ts-mono">2</span>/<span class="ts-mono">3</span>
              tabs · <span class="ts-mono">R</span> refresh ·
              <span class="ts-mono">D</span> dashboard ·
              <span class="ts-mono">Esc</span> collapse
            </div>
          </div>
        </div>

        <div class="ts-set-note">
          <Icon name="folder" size={14} />
          <div>
            <div class="ts-set-note-k">Pricing source</div>
            <div class="ts-set-note-v ts-mono">~/.config/tokenscope/pricing.toml</div>
          </div>
        </div>
        <div class="ts-set-note">
          <Icon name="cpu" size={14} />
          <div>
            <div class="ts-set-note-k">Data source</div>
            <div class="ts-set-note-v ts-mono">
              ~/.claude/ · local only, never uploaded
            </div>
          </div>
        </div>
      </div>
    {/if}
  </div>

  <div class="ts-pop-foot">
    <button
      class="ts-foot-btn"
      onclick={() => load(false)}
      disabled={loading}
      title="Refresh"
    >
      <Icon name="refresh" size={13} />
      updated {fmtRelTime(lastLoadedAt, now)}
    </button>
    <button class="ts-foot-btn ts-foot-primary" onclick={openDashboard}>
      <Icon name="expand" size={13} />
      Dashboard
    </button>
  </div>
</div>

<style>
  .ts-popover {
    position: relative;
    width: 100vw;
    height: 100vh;
    background: var(--ts-bg-popover);
    color: var(--ts-text-1);
    font-size: 13px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* hero */
  .ts-hero {
    padding: 16px 18px 14px;
    border-bottom: 1px solid var(--ts-border);
    flex-shrink: 0;
  }
  .ts-hero-empty {
    color: var(--ts-text-2);
  }
  .ts-hero-top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
  }
  .ts-hero-label {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.6px;
    color: var(--ts-text-3);
    margin-bottom: 4px;
  }
  .ts-hero-label-r {
    text-align: right;
  }
  .ts-hero-cost {
    font-size: 34px;
    font-weight: 650;
    line-height: 1;
    letter-spacing: -0.5px;
  }
  .ts-hero-quota-sep {
    font-size: 0.55em;
    color: var(--ts-text-3);
    font-weight: 500;
    margin-left: 4px;
    letter-spacing: 0;
  }
  .ts-hero-proj {
    font-size: 12px;
    color: var(--ts-text-2);
    margin-top: 5px;
  }
  .ts-hero-burn {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    min-width: 130px;
  }
  .ts-hero-sparkrow {
    margin-top: 2px;
  }
  .ts-hero-burnval-sm {
    font-size: 13px;
    font-weight: 600;
    margin-top: 4px;
    align-self: flex-end;
  }
  .ts-hero-unit {
    font-size: 0.62em;
    color: var(--ts-text-3);
    font-weight: 500;
    margin-left: 1px;
  }
  .ts-hero-reset {
    margin-top: 14px;
  }
  .ts-hero-resetbar {
    height: 4px;
    border-radius: 3px;
    background: var(--ts-surface);
    overflow: hidden;
  }
  .ts-hero-resetfill {
    height: 100%;
    border-radius: 3px;
    background: linear-gradient(
      90deg,
      var(--ts-accent),
      color-mix(in oklab, var(--ts-accent), white 18%)
    );
  }
  .ts-hero-resetmeta {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--ts-text-2);
    margin-top: 6px;
  }

  /* tabs */
  .ts-pop-tabs {
    display: flex;
    gap: 2px;
    padding: 8px 12px 0;
    border-bottom: 1px solid var(--ts-border);
    flex-shrink: 0;
  }
  .ts-tab {
    appearance: none;
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: var(--ts-font);
    font-size: 13px;
    font-weight: 500;
    color: var(--ts-text-2);
    padding: 7px 12px 9px;
    position: relative;
    border-radius: 6px 6px 0 0;
    transition: color 0.12s;
  }
  .ts-tab:hover {
    color: var(--ts-text-1);
  }
  .ts-tab.is-on {
    color: var(--ts-text-1);
    font-weight: 600;
  }
  .ts-tab.is-on::after {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: -1px;
    height: 2px;
    border-radius: 2px;
    background: var(--ts-accent);
  }

  /* scroll body */
  .ts-pop-scroll {
    flex: 1;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .ts-pop-scroll::-webkit-scrollbar {
    width: 9px;
  }
  .ts-pop-scroll::-webkit-scrollbar-thumb {
    background: var(--ts-border-2);
    border-radius: 6px;
    border: 2px solid var(--ts-bg-popover);
  }
  .ts-tabbody {
    padding: 12px;
  }
  .ts-empty-state {
    padding: 30px 12px;
    text-align: center;
    color: var(--ts-text-3);
    font-size: 13px;
  }
  .ts-err {
    color: var(--ts-tier-critical);
  }

  /* footer */
  .ts-pop-foot {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    border-top: 1px solid var(--ts-border);
    background: var(--ts-surface-2);
    flex-shrink: 0;
  }
  .ts-foot-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: var(--ts-font);
    font-size: 12px;
    color: var(--ts-text-2);
    padding: 4px 8px;
    border-radius: 6px;
    transition: 0.12s;
  }
  .ts-foot-btn:hover {
    background: var(--ts-surface-hi);
    color: var(--ts-text-1);
  }
  .ts-foot-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .ts-foot-primary {
    color: var(--ts-text-1);
    font-weight: 550;
  }
  .ts-foot-primary:hover {
    background: var(--ts-accent-weak);
    color: var(--ts-accent);
  }

  /* sessions cards */
  .ts-sessions {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .ts-card {
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border);
    border-radius: 9px;
    padding: 11px 12px;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s;
  }
  .ts-card:hover {
    background: var(--ts-surface);
    border-color: var(--ts-border-2);
  }
  .ts-card.is-open {
    background: var(--ts-surface);
    border-color: var(--ts-accent-line);
  }
  .ts-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .ts-card-id {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }
  .ts-card-name {
    font-weight: 550;
    font-size: 13.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ts-card-name.is-mono {
    font-family: var(--ts-mono);
    color: var(--ts-text-2);
    font-size: 12.5px;
  }
  .ts-card-cost {
    font-weight: 600;
    font-size: 14px;
  }
  .ts-card-unit {
    font-size: 0.72em;
    color: var(--ts-text-3);
    font-weight: 500;
    margin-left: 2px;
  }
  .ts-card-ctx {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-top: 9px;
  }
  .ts-card-ctxpct {
    font-size: 11px;
    font-weight: 600;
    width: 30px;
    text-align: right;
  }
  .ts-card-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
    font-size: 11.5px;
    color: var(--ts-text-2);
  }
  .ts-card-meta .ts-mono {
    color: var(--ts-text-2);
  }
  .ts-meta-dot {
    color: var(--ts-text-3);
  }
  .ts-card-time {
    margin-left: auto;
    color: var(--ts-text-3);
  }
  .ts-sessions-foot {
    text-align: center;
    font-size: 11.5px;
    color: var(--ts-text-3);
    padding: 10px 4px 4px;
  }

  /* card detail */
  .ts-card-detail {
    margin-top: 11px;
    padding-top: 11px;
    border-top: 1px solid var(--ts-border);
    cursor: default;
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
  .ts-detail-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
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
  .ts-detail-rows {
    margin-top: 10px;
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

  /* active block */
  .ts-activeblock {
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border);
    border-radius: 11px;
    padding: 15px 16px;
  }
  .ts-ab-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .ts-ab-live {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.6px;
    color: var(--ts-st-active);
  }
  .ts-ab-livedot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ts-st-active);
    box-shadow: 0 0 0 0 rgba(74, 222, 128, 0.5);
    animation: ts-livepulse 2s infinite;
  }
  @keyframes ts-livepulse {
    0% {
      box-shadow: 0 0 0 0 rgba(74, 222, 128, 0.45);
    }
    70% {
      box-shadow: 0 0 0 6px rgba(74, 222, 128, 0);
    }
    100% {
      box-shadow: 0 0 0 0 rgba(74, 222, 128, 0);
    }
  }
  .ts-ab-window {
    font-size: 11.5px;
    color: var(--ts-text-2);
  }
  .ts-ab-costrow {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-top: 10px;
  }
  .ts-ab-cost {
    font-size: 40px;
    font-weight: 650;
    line-height: 1;
    letter-spacing: -0.6px;
  }
  .ts-ab-quota-sep {
    font-size: 0.5em;
    color: var(--ts-text-3);
    font-weight: 500;
    margin-left: 4px;
    letter-spacing: 0;
  }
  .ts-ab-block-primary {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ts-ab-shadow {
    font-size: 11px;
    color: var(--ts-text-3);
    font-weight: 500;
  }
  .ts-text3 {
    color: var(--ts-text-3);
  }
  .ts-ab-proj {
    text-align: right;
  }
  .ts-ab-projval {
    font-size: 17px;
    font-weight: 600;
    color: var(--ts-text-1);
  }
  .ts-ab-projlbl {
    font-size: 10.5px;
    color: var(--ts-text-3);
    margin-top: 2px;
  }
  .ts-ab-resetbar {
    height: 5px;
    border-radius: 3px;
    background: var(--ts-surface-hi);
    overflow: hidden;
    margin-top: 14px;
  }
  .ts-ab-resetfill {
    height: 100%;
    border-radius: 3px;
    background: linear-gradient(
      90deg,
      var(--ts-accent),
      color-mix(in oklab, var(--ts-accent), white 20%)
    );
  }
  .ts-ab-resetmeta {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--ts-text-2);
    margin-top: 6px;
  }
  .ts-ab-stats {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 10px;
    margin-top: 16px;
    padding-top: 14px;
    border-top: 1px solid var(--ts-border);
  }
  .ts-ab-stat {
    min-width: 0;
  }
  .ts-ab-statlbl {
    font-size: 9.5px;
    font-weight: 600;
    letter-spacing: 0.5px;
    color: var(--ts-text-3);
    margin-bottom: 6px;
  }
  .ts-ab-statval {
    font-size: 18px;
    font-weight: 600;
  }
  .ts-ab-statval-sm {
    font-size: 10.5px;
    color: var(--ts-text-3);
    margin-top: 3px;
  }
  .ts-ab-models {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
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

  .ts-section-label {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.5px;
    color: var(--ts-text-3);
    margin: 18px 4px 8px;
  }
  .ts-blocktable {
    display: flex;
    flex-direction: column;
  }
  .ts-bt-head {
    display: grid;
    grid-template-columns: 1.6fr 0.7fr 0.9fr 0.9fr;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.4px;
    color: var(--ts-text-3);
    padding: 0 6px 7px;
  }
  .ts-bt-r {
    text-align: right;
  }
  .ts-bt-row {
    display: grid;
    grid-template-columns: 1.6fr 0.7fr 0.9fr 0.9fr;
    font-size: 12px;
    padding: 8px 6px;
    border-radius: 6px;
    align-items: center;
  }
  .ts-bt-row:hover {
    background: var(--ts-surface-2);
  }
  .ts-bt-row:not(:last-child) {
    border-bottom: 1px solid var(--ts-border);
  }
  .ts-bt-cost {
    font-weight: 600;
  }
  .ts-text2 {
    color: var(--ts-text-2);
  }

  /* settings */
  .ts-settings {
    padding: 16px;
  }
  .ts-set-label {
    display: block;
    font-size: 13px;
    font-weight: 550;
    margin-bottom: 9px;
  }
  .ts-set-inputrow {
    display: flex;
    align-items: center;
    gap: 0;
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border-2);
    border-radius: 8px;
    padding: 0 12px;
    height: 38px;
    max-width: 230px;
  }
  .ts-set-inputrow:focus-within {
    border-color: var(--ts-accent);
    box-shadow: 0 0 0 3px var(--ts-accent-weak);
  }
  .ts-set-prefix {
    color: var(--ts-text-2);
    font-size: 15px;
  }
  .ts-set-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--ts-text-1);
    font-family: var(--ts-font);
    font-size: 15px;
    font-weight: 550;
    padding: 0 4px;
    width: 100%;
  }
  .ts-set-suffix {
    color: var(--ts-text-3);
    font-size: 12px;
    white-space: nowrap;
  }
  .ts-set-hint {
    font-size: 11.5px;
    color: var(--ts-text-3);
    margin-top: 8px;
    line-height: 1.5;
  }
  .ts-set-divider {
    height: 1px;
    background: var(--ts-border);
    margin: 20px 0;
  }
  .ts-set-note {
    display: flex;
    gap: 11px;
    align-items: flex-start;
    padding: 9px 0;
    color: var(--ts-text-2);
  }
  .ts-set-note > :global(svg) {
    margin-top: 1px;
    color: var(--ts-text-3);
  }
  .ts-set-note-k {
    font-size: 12.5px;
    color: var(--ts-text-1);
    font-weight: 500;
  }
  .ts-set-note-v {
    font-size: 11.5px;
    color: var(--ts-text-2);
    margin-top: 2px;
  }

  /* hide number input spinners */
  .ts-set-input::-webkit-inner-spin-button,
  .ts-set-input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  /* plan segmented control */
  .ts-plan-seg {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 1px;
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border-2);
    border-radius: 8px;
    padding: 2px;
    max-width: 320px;
  }
  .ts-plan-opt {
    background: transparent;
    border: none;
    cursor: pointer;
    font-family: var(--ts-font);
    font-size: 12px;
    color: var(--ts-text-2);
    padding: 6px 4px;
    border-radius: 6px;
    transition: 0.1s;
  }
  .ts-plan-opt:hover {
    color: var(--ts-text-1);
  }
  .ts-plan-opt.is-on {
    background: var(--ts-surface-hi);
    color: var(--ts-text-1);
    font-weight: 550;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }

  /* first-run modal */
  .ts-firstrun-scrim {
    position: absolute;
    inset: 0;
    background: var(--ts-scrim);
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }
  .ts-firstrun {
    width: 100%;
    background: var(--ts-bg-content);
    border: 1px solid var(--ts-border);
    border-radius: var(--ts-r-xl);
    padding: 18px 18px 14px;
    box-shadow: var(--ts-pop-shadow);
  }
  .ts-firstrun-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--ts-text-1);
  }
  .ts-firstrun-sub {
    font-size: 12px;
    color: var(--ts-text-2);
    margin-top: 4px;
    margin-bottom: 12px;
  }
  .ts-firstrun-plans {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .ts-firstrun-plan {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    text-align: left;
    background: var(--ts-surface-2);
    border: 1px solid var(--ts-border);
    border-radius: var(--ts-r-md);
    padding: 10px 12px;
    color: var(--ts-text-1);
    cursor: pointer;
  }
  .ts-firstrun-plan:hover {
    background: var(--ts-surface);
    border-color: var(--ts-accent-line);
  }
  .ts-firstrun-plan-name {
    font-weight: 600;
    font-size: 13px;
  }
  .ts-firstrun-plan-desc {
    font-size: 11px;
    color: var(--ts-text-3);
    margin-top: 2px;
  }
  .ts-firstrun-foot {
    margin-top: 12px;
    font-size: 11px;
    color: var(--ts-text-3);
    text-align: center;
  }
</style>
