<script lang="ts" module>
  // Zajil's own coloured icons (the owner's request: the toolbar, the tree
  // and the list with more life, as desktop download managers have):
  // filled shapes with a soft gradient and a white glyph or a small badge.
  // Drawn for Zajil, from no other program's icons.

  /** The colours, light and dark at once: each shape's top and bottom. */
  const TONES = {
    green: ["#46d36c", "#169a3d"],
    red: ["#ff6b61", "#d42f2c"],
    amber: ["#ffc54d", "#e08a06"],
    blue: ["#5aa0ff", "#1d63d8"],
    purple: ["#ad85ff", "#6c3ce0"],
    brown: ["#f1a454", "#b8641b"],
    dark: ["#5d7584", "#2a3a44"],
    grey: ["#c3cad4", "#8a94a3"],
    yellow: ["#ffd361", "#e59c1c"],
    slate: ["#8494a8", "#56667a"],
  } as const;
  type Tone = keyof typeof TONES;

  let next = 0;

  export { typeIcon, nameIcon } from "../fileIcon";
  export { TONES };
  export type { Tone };
</script>

<script lang="ts">
  let { name, size = 32 }: { name: string; size?: number } = $props();
  // Each drawing's gradients by its own ids: a hidden icon's would leave
  // the others without theirs.
  const id = `ci${next++}`;
  const g = (tone: Tone) => `url(#${id}-${tone})`;
  const used: Tone[] = ["green", "red", "amber", "blue", "purple", "brown", "dark", "grey", "yellow", "slate"];
</script>

<svg class="ci" width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
  <defs>
    {#each used as tone (tone)}
      <linearGradient id="{id}-{tone}" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color={TONES[tone][0]} />
        <stop offset="1" stop-color={TONES[tone][1]} />
      </linearGradient>
    {/each}
  </defs>
  {#if name === "add"}
    <circle cx="16" cy="16" r="13.5" fill={g("green")} />
    <path d="M16 9v14M9 16h14" stroke="#fff" stroke-width="3.6" stroke-linecap="round" />
  {:else if name === "resume"}
    <path d="M9 6.4c0-1.7 1.9-2.7 3.3-1.8l14.2 9.6c1.2.8 1.2 2.6 0 3.4L12.3 27.2C10.9 28.1 9 27.1 9 25.4z" fill={g("green")} />
  {:else if name === "pause"}
    <rect x="7" y="5" width="7" height="22" rx="2.2" fill={g("amber")} />
    <rect x="18" y="5" width="7" height="22" rx="2.2" fill={g("amber")} />
  {:else if name === "pause-all"}
    <rect x="9" y="3" width="20" height="20" rx="5" fill={TONES.red[1]} opacity="0.5" />
    <rect x="3" y="9" width="20" height="20" rx="5" fill={g("red")} />
    <rect x="8.8" y="14" width="3.2" height="10" rx="1.1" fill="#fff" />
    <rect x="14" y="14" width="3.2" height="10" rx="1.1" fill="#fff" />
  {:else if name === "delete" || name === "delete-done"}
    <g transform={name === "delete-done" ? "translate(-2.5 -2) scale(0.9)" : undefined}>
      <path d="M12.6 6.6V5c0-1 .8-1.8 1.8-1.8h3.2c1 0 1.8.8 1.8 1.8v1.6" fill="none" stroke={TONES.red[1]} stroke-width="2" />
      <rect x="5.5" y="6.4" width="21" height="3.6" rx="1.6" fill={TONES.red[1]} />
      <path d="M7.8 11.6h16.4l-1.3 14.4c-.1 1.4-1.3 2.5-2.7 2.5h-8.4c-1.4 0-2.6-1.1-2.7-2.5z" fill={g("red")} />
      <path d="M13 15.5v8.5M19 15.5v8.5" stroke="#fff" stroke-width="2" stroke-linecap="round" opacity="0.9" />
    </g>
    {#if name === "delete-done"}
      <circle cx="23.5" cy="23.5" r="7.5" fill={g("green")} stroke="var(--surface)" stroke-width="1.5" />
      <path d="M20.2 23.6l2.3 2.3 4.2-4.6" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" />
    {/if}
  {:else if name === "settings"}
    <path d="M5 8.5h22M5 16h22M5 23.5h22" stroke={TONES.slate[0]} stroke-width="2.6" stroke-linecap="round" />
    <circle cx="20" cy="8.5" r="3.8" fill={g("blue")} stroke="var(--surface)" stroke-width="1.4" />
    <circle cx="11" cy="16" r="3.8" fill={g("amber")} stroke="var(--surface)" stroke-width="1.4" />
    <circle cx="18" cy="23.5" r="3.8" fill={g("green")} stroke="var(--surface)" stroke-width="1.4" />
  {:else if name === "schedule"}
    <circle cx="16" cy="16.5" r="13" fill={g("blue")} />
    <circle cx="16" cy="16.5" r="9.6" fill="#fff" />
    <path d="M16 10.5v6.3l4 2.6" fill="none" stroke={TONES.blue[1]} stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="16" cy="16.8" r="1.4" fill={TONES.blue[1]} />
  {:else if name === "run-queue" || name === "halt-queue"}
    <rect x="3" y="5" width="18" height="3.6" rx="1.8" fill={g("slate")} />
    <rect x="3" y="12" width="18" height="3.6" rx="1.8" fill={g("slate")} />
    <rect x="3" y="19" width="11" height="3.6" rx="1.8" fill={g("slate")} />
    {#if name === "run-queue"}
      <circle cx="23.5" cy="23.5" r="7.5" fill={g("green")} stroke="var(--surface)" stroke-width="1.5" />
      <path d="M21.6 20.2v6.6l5.3-3.3z" fill="#fff" stroke="#fff" stroke-width="0.8" stroke-linejoin="round" />
    {:else}
      <circle cx="23.5" cy="23.5" r="7.5" fill={g("red")} stroke="var(--surface)" stroke-width="1.5" />
      <rect x="20.7" y="20.7" width="5.6" height="5.6" rx="1.1" fill="#fff" />
    {/if}
  {:else if name === "all"}
    <path d="M3 8.6A3 3 0 0 1 6 5.6h6.4l2.8 3H26a3 3 0 0 1 3 3V24a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" fill={TONES.yellow[1]} />
    <path d="M3 12.5h26V24a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" fill={g("yellow")} />
  {:else if name === "queue" || name === "queue-timed"}
    <path d="M2 8.6A3 3 0 0 1 5 5.6h6.4l2.8 3H24a3 3 0 0 1 3 3V24a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z" fill={TONES.yellow[1]} />
    <path d="M2 12.5h25V24a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z" fill={g("yellow")} />
    {#if name === "queue"}
      <circle cx="24" cy="23" r="7" fill={g("green")} stroke="var(--side)" stroke-width="1.5" />
      <path d="M22.3 20v6l4.9-3z" fill="#fff" stroke="#fff" stroke-width="0.8" stroke-linejoin="round" />
    {:else}
      <circle cx="24" cy="23" r="7" fill={g("blue")} stroke="var(--side)" stroke-width="1.5" />
      <path d="M24 19.6v3.6l2.3 1.4" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    {/if}
  {:else if name === "unfinished"}
    <path d="M12 4.5h8v10.5h5.2L16 26.5 6.8 15H12z" fill={g("blue")} stroke={TONES.blue[1]} stroke-width="1.6" stroke-linejoin="round" />
  {:else if name === "finished"}
    <path d="M5.5 16.5l7 7L26.5 8.5" fill="none" stroke={g("green")} stroke-width="5" stroke-linecap="round" stroke-linejoin="round" />
  {:else if name === "waiting"}
    <circle cx="16" cy="16" r="12.5" fill={g("amber")} />
    <path d="M16 9.5v7l4.4 2.8" fill="none" stroke="#fff" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" />
  {:else if name === "s-active"}
    <circle cx="16" cy="16" r="13" fill={g("blue")} />
    <path d="M16 8.5v13M10.5 16l5.5 5.5 5.5-5.5" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
  {:else if name === "s-paused"}
    <circle cx="16" cy="16" r="13" fill={g("amber")} />
    <path d="M12.5 10v12M19.5 10v12" stroke="#fff" stroke-width="3.4" stroke-linecap="round" />
  {:else if name === "s-done"}
    <circle cx="16" cy="16" r="13" fill={g("green")} />
    <path d="M10 16.5l4.2 4.2L22.5 12" fill="none" stroke="#fff" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round" />
  {:else if name === "s-failed"}
    <circle cx="16" cy="16" r="13" fill={g("red")} />
    <path d="M16 9v8.5" stroke="#fff" stroke-width="3.4" stroke-linecap="round" />
    <circle cx="16" cy="22.4" r="2" fill="#fff" />
  {:else if name === "s-waiting"}
    <circle cx="16" cy="16" r="13" fill={g("grey")} />
    <path d="M16 9.5v7l4.4 2.8" fill="none" stroke="#fff" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" />
  {:else if name === "t-video"}
    <rect x="3" y="6" width="26" height="20" rx="3.5" fill={g("blue")} />
    {#each [8.5, 13.2, 17.9, 22.6] as y (y)}
      <rect x="5" y={y - 1.2} width="2.4" height="2.4" rx="0.6" fill="#fff" opacity="0.8" />
      <rect x="24.6" y={y - 1.2} width="2.4" height="2.4" rx="0.6" fill="#fff" opacity="0.8" />
    {/each}
    <path d="M13 11.2v9.6l7.6-4.8z" fill="#fff" stroke="#fff" stroke-width="1" stroke-linejoin="round" />
  {:else if name === "t-music"}
    <circle cx="16" cy="16" r="13.5" fill={g("purple")} />
    <circle cx="13" cy="21.3" r="3.4" fill="#fff" />
    <rect x="15" y="8" width="2.3" height="13.5" rx="1" fill="#fff" />
    <path d="M17.3 8c2.6.6 5 2.3 5 5.2-1.2-1.4-2.9-2-5-2z" fill="#fff" />
  {:else if name === "t-images"}
    <rect x="3" y="5" width="26" height="22" rx="3.5" fill={g("green")} />
    <circle cx="11" cy="12" r="2.8" fill="#fff6c4" />
    <path d="M5.5 24.6l7.2-8 4.8 5.2 3.6-3.6 5.4 6.4z" fill="#fff" opacity="0.92" />
  {:else if name === "t-archives"}
    <rect x="5" y="11" width="22" height="16.5" rx="2.2" fill={g("brown")} />
    <rect x="3.5" y="5.5" width="25" height="7" rx="2" fill={TONES.brown[1]} />
    <rect x="13" y="12.8" width="6" height="5.4" rx="1.2" fill="#fff" opacity="0.9" />
  {:else if name === "t-programs"}
    <rect x="3" y="5" width="26" height="22" rx="3" fill={g("dark")} />
    <rect x="5.4" y="11.5" width="21.2" height="13" rx="1.3" fill="#eef3f6" />
    <circle cx="7.4" cy="8.3" r="1.1" fill="#ff6b61" />
    <circle cx="10.6" cy="8.3" r="1.1" fill="#ffc54d" />
    <circle cx="13.8" cy="8.3" r="1.1" fill="#46d36c" />
    <path d="M9 15.5l3.2 2.6L9 20.7M14.2 21h6" fill="none" stroke="#2a3a44" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
  {:else}
    <!-- A page: blue with lines for a document, grey for any file. -->
    {@const tone = name === "t-documents" ? "blue" : "grey"}
    <path d="M8.5 3h10.8L26 9.7V27a2 2 0 0 1-2 2H8.5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z" fill={g(tone)} />
    <path d="M19.3 3v4.7a2 2 0 0 0 2 2H26" fill="#fff" opacity="0.5" />
    {#if name === "t-documents"}
      <path d="M10.5 15h11M10.5 19h11M10.5 23h7" stroke="#fff" stroke-width="1.9" stroke-linecap="round" />
    {/if}
  {/if}
</svg>

<style>
  .ci {
    flex: none;
    display: block;
  }
</style>
