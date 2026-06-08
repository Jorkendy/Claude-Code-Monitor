<script lang="ts">
  let {
    data,
    width = 120,
    height = 30,
    stroke = "var(--ts-accent)",
    fill = true,
    strokeWidth = 1.6,
  }: {
    data: number[];
    width?: number;
    height?: number;
    stroke?: string;
    fill?: boolean;
    strokeWidth?: number;
  } = $props();

  const safe = $derived(data.length >= 2 ? data : [0, 0]);
  const max = $derived(Math.max(...safe, 0.0001));
  const pts = $derived(
    safe.map((v, i) => {
      const x = (i / (safe.length - 1)) * width;
      const y = height - (v / max) * (height - 4) - 2;
      return [x, y] as [number, number];
    }),
  );
  const line = $derived(
    pts.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)}`).join(" "),
  );
  const area = $derived(`${line} L${width} ${height} L0 ${height} Z`);
  const last = $derived(pts[pts.length - 1] ?? [0, 0]);
  // Stable gradient id per instance.
  const gid = $derived("spk" + Math.floor(Math.random() * 1e9).toString(36));
</script>

<svg
  {width}
  {height}
  viewBox="0 0 {width} {height}"
  style="display:block;overflow:visible;"
>
  {#if fill}
    <defs>
      <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color={stroke} stop-opacity="0.22" />
        <stop offset="100%" stop-color={stroke} stop-opacity="0" />
      </linearGradient>
    </defs>
    <path d={area} fill="url(#{gid})" />
  {/if}
  <path
    d={line}
    fill="none"
    {stroke}
    stroke-width={strokeWidth}
    stroke-linejoin="round"
    stroke-linecap="round"
  />
  <circle cx={last[0]} cy={last[1]} r="2.4" fill={stroke} />
</svg>
