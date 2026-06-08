<script lang="ts">
  import "$lib/styles/tokens.css";
  let { children } = $props();
</script>

{@render children?.()}

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: var(--ts-bg-popover);
    color: var(--ts-text-1);
    font-family: var(--ts-font);
    font-size: 13px;
    line-height: 1.4;
    -webkit-font-smoothing: antialiased;
    text-rendering: optimizeLegibility;
  }
  :global(*, *::before, *::after) {
    box-sizing: border-box;
  }
  :global(button) {
    font-family: inherit;
  }
</style>
