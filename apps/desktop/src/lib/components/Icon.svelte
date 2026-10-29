<script lang="ts">
  import { lucide } from "../lucide";

  // Simple line icons drawn for Zajil (not taken from any other program),
  // and Lucide's (M5.5, `../lucide.ts`, ISC) by their Lucide names.
  const paths: Record<string, string> = {
    plus: '<path d="M12 5v14M5 12h14"/>',
    pause: '<path d="M9 6v12M15 6v12"/>',
    play: '<path d="M8 5.5v13l10.5-6.5z"/>',
    trash: '<path d="M4 7h16M9 7V4.5h6V7M6.5 7l1 12.5h9l1-12.5"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2"/>',
    folder: '<path d="M3.5 6.5h6l2 2h9v10h-17z"/>',
    sliders:
      '<path d="M4 7h9M17 7h3M4 12h3M11 12h9M4 17h11M19 17h1"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="17" r="2"/>',
    link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3A4 4 0 0 0 13 5.3l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1-1"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7"/>',
    x: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
    alert: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5v5.5M12 16.2v.3"/>',
    down: '<path d="M12 4v11M6.5 10l5.5 5.5 5.5-5.5M5 20h14"/>',
    all: '<path d="M4 6.5h16M4 12h16M4 17.5h16"/>',
    tag: '<path d="M3.5 11.5V3.5h8l9 9-8 8z"/><circle cx="8" cy="8" r="1.3"/>',
    queue: '<path d="M4 6.5h16M4 12h16M4 17.5h9"/><path d="M17 15.5l2.5 2-2.5 2"/>',
    back: '<path d="M14.5 6l-6 6 6 6"/>',
    gauge: '<path d="M4.5 16a7.5 7.5 0 1 1 15 0"/><path d="M12 16l3.5-4.5"/>',
    open: '<path d="M14 4.5h5.5V10M19.5 4.5L12 12M10 5.5H4.5v14h14V14"/>',
    globe:
      '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.5 2.6 3.7 5.4 3.7 8.5s-1.2 5.9-3.7 8.5M12 3.5c-2.5 2.6-3.7 5.4-3.7 8.5s1.2 5.9 3.7 8.5"/>',
    key: '<circle cx="8" cy="15" r="3.5"/><path d="M10.5 12.5L19 4M16 7l2.5 2.5M14 9l2 2"/>',
    sound: '<path d="M4.5 9.5h3.5l4.5-4v13l-4.5-4H4.5z"/><path d="M16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11"/>',
    sun: '<circle cx="12" cy="12" r="3.5"/><path d="M12 3.5v2M12 18.5v2M3.5 12h2M18.5 12h2M6 6l1.4 1.4M16.6 16.6L18 18M6 18l1.4-1.4M16.6 7.4L18 6"/>',
  };
  let { name, size = 18, stroke = 1.8 }: { name: string; size?: number; stroke?: number } = $props();
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  aria-hidden="true"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
>
  {@html paths[name] ?? lucide[name] ?? ""}
</svg>

<style>
  .icon {
    flex: none;
  }
</style>
