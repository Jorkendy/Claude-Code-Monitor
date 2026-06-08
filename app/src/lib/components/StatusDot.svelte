<script lang="ts">
  let {
    status,
    size = 8,
  }: { status: "active" | "idle" | "inactive"; size?: number } = $props();
</script>

{#if status === "inactive"}
  <span
    class="ts-dot is-hollow"
    title="inactive"
    style="width:{size}px;height:{size}px;border-color:var(--ts-st-inactive);"
  ></span>
{:else}
  <span
    class="ts-dot"
    title={status}
    style="width:{size}px;height:{size}px;"
  >
    {#if status === "active"}
      <span class="pulse" style="background:var(--ts-st-active);"></span>
    {/if}
    <span
      class="fill"
      style="background:var(--ts-st-{status});"
    ></span>
  </span>
{/if}

<style>
  .ts-dot {
    position: relative;
    display: inline-block;
    flex-shrink: 0;
  }
  .ts-dot.is-hollow {
    border-radius: 50%;
    border: 1.5px solid;
    background: transparent;
  }
  .fill {
    position: absolute;
    inset: 0;
    border-radius: 50%;
  }
  .pulse {
    position: absolute;
    inset: -3px;
    border-radius: 50%;
    opacity: 0.35;
    animation: ts-pulse 2s ease-out infinite;
  }
  @keyframes ts-pulse {
    0% {
      transform: scale(0.7);
      opacity: 0.5;
    }
    70%,
    100% {
      transform: scale(1.7);
      opacity: 0;
    }
  }
</style>
