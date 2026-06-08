<script lang="ts">
  import { contextTier } from "$lib/format";
  let {
    pct,
    height = 4,
    showLabel = false,
  }: { pct: number; height?: number; showLabel?: boolean } = $props();

  const tier = $derived(contextTier(pct));
  const w = $derived(Math.min(100, Math.max(2, pct)));
</script>

<div class="ts-ctxbar-wrap">
  <div class="ts-ctxbar" style="height:{height}px;">
    <div
      class="ts-ctxbar-fill"
      style="width:{w}%;background:var(--ts-tier-{tier});"
    ></div>
  </div>
  {#if showLabel}
    <span class="ts-ctxbar-label ts-tnum" style="color:var(--ts-tier-{tier});">
      {Math.round(pct)}%
    </span>
  {/if}
</div>

<style>
  .ts-ctxbar-wrap {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
  }
  .ts-ctxbar {
    flex: 1;
    background: var(--ts-surface-hi);
    border-radius: 3px;
    overflow: hidden;
    min-width: 0;
  }
  .ts-ctxbar-fill {
    height: 100%;
    border-radius: 3px;
    transition: width 0.3s;
  }
  .ts-ctxbar-label {
    font-size: 11px;
    font-weight: 600;
  }
</style>
