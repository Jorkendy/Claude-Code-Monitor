<script lang="ts">
  import Icon from "./Icon.svelte";

  let {
    text,
    title = "Copy",
    size = 12,
  }: { text: string; title?: string; size?: number } = $props();

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function onClick(e: MouseEvent) {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => (copied = false), 1200);
    } catch {
      // Clipboard API can fail when not focused; fall back via execCommand.
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => (copied = false), 1200);
    }
  }
</script>

<button
  class="ts-copy"
  class:is-copied={copied}
  onclick={onClick}
  title={copied ? "Copied" : title}
  aria-label={copied ? "Copied" : title}
>
  <Icon name={copied ? "check" : "copy"} {size} strokeWidth={1.8} />
</button>

<style>
  .ts-copy {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    margin-left: 6px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--ts-text-3);
    cursor: pointer;
    flex-shrink: 0;
    transition: 0.12s;
  }
  .ts-copy:hover {
    background: var(--ts-surface-hi);
    color: var(--ts-text-1);
  }
  .ts-copy.is-copied {
    color: var(--ts-st-active);
  }
</style>
