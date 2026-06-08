<script lang="ts">
  import "$lib/styles/tokens.css";
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  let { children } = $props();

  // Theme override is shared across popover + dashboard windows. Listening
  // for `data-changed` keeps a window in sync when the other one toggles it.
  function applyTheme(theme: string) {
    const root = document.documentElement;
    if (theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", theme);
  }
  async function refreshTheme() {
    try {
      const s = await invoke<{ theme: string }>("get_settings");
      applyTheme(s.theme ?? "system");
    } catch {
      /* ignore — leave whatever's already set */
    }
  }
  let unlisten: UnlistenFn | undefined;
  onMount(async () => {
    await refreshTheme();
    unlisten = await listen("data-changed", refreshTheme);
  });
  onDestroy(() => unlisten?.());
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
