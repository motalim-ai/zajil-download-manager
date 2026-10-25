<script lang="ts" module>
  /** What a toolbar button does (M5.5, the design of #83). */
  export type ToolAction =
    | "add"
    | "resume"
    | "pause"
    | "pause-all"
    | "delete"
    | "delete-done"
    | "settings"
    | "schedule"
    | "run-queue"
    | "halt-queue";
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import { shortcut } from "../keys";

  let {
    search = $bindable(),
    can,
    onaction,
  }: {
    search: string;
    /** Whether a button has something to act on now. */
    can: (action: ToolAction) => boolean;
    onaction: (action: ToolAction) => void;
  } = $props();

  // A colour for each kind of action (owner's decision on #83): green to
  // run, amber to stop, red to delete, grey for settings and the schedule,
  // blue to add a link. A line between the groups.
  const groups: { action: ToolAction; icon: string; key: string; kind: string }[][] = [
    [{ action: "add", icon: "link-2", key: "tb-add", kind: "add" }],
    [
      { action: "resume", icon: "play", key: "tb-resume", kind: "go" },
      { action: "pause", icon: "pause", key: "tb-pause", kind: "stop" },
      { action: "pause-all", icon: "octagon-pause", key: "tb-pause-all", kind: "stop" },
    ],
    [
      { action: "delete", icon: "trash-2", key: "tb-delete", kind: "del" },
      { action: "delete-done", icon: "list-x", key: "tb-delete-done", kind: "del" },
    ],
    [
      { action: "settings", icon: "settings", key: "tb-settings", kind: "calm" },
      { action: "schedule", icon: "calendar-clock", key: "tb-schedule", kind: "calm" },
    ],
    [
      { action: "run-queue", icon: "list-start", key: "tb-run-queue", kind: "go" },
      { action: "halt-queue", icon: "list-end", key: "tb-halt-queue", kind: "stop" },
    ],
  ];
</script>

<div class="toolbar" role="toolbar" aria-label={locale.t("tb-toolbar")}>
  {#each groups as group, g (g)}
    {#if g > 0}<span class="sep" aria-hidden="true"></span>{/if}
    {#each group as b (b.action)}
      <button id="tb-{b.action}" class="tb {b.kind}" disabled={!can(b.action)} onclick={() => onaction(b.action)}>
        <span class="tile"><Icon name={b.icon} size={22} stroke={1.7} /></span>
        <span class="label">{locale.t(b.key)}</span>
      </button>
    {/each}
  {/each}
  <label class="search">
    <Icon name="search" size={16} />
    <input id="search" type="search" bind:value={search} placeholder={locale.t("search-placeholder")} aria-label={locale.t("search-placeholder")} />
    <kbd aria-hidden="true">{shortcut("mod+F")}</kbd>
  </label>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    overflow-x: auto;
  }
  .tb {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    min-width: 74px;
    padding: 6px 6px 4px;
    border-radius: 8px;
    border: 1px solid transparent;
    background: none;
    cursor: pointer;
    font-size: 12px;
    color: var(--ink);
    line-height: 1.3;
  }
  .tb:hover:not(:disabled) {
    background: var(--sunken);
    border-color: var(--line);
  }
  .tb:disabled {
    cursor: default;
    opacity: 0.45;
  }
  .tile {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    display: grid;
    place-items: center;
    background: var(--feather-bg);
    color: var(--accent);
  }
  .go .tile {
    background: var(--green-bg);
    color: var(--green);
  }
  .stop .tile {
    background: var(--amber-bg);
    color: var(--amber);
  }
  .del .tile {
    background: var(--red-bg);
    color: var(--red);
  }
  .calm .tile {
    background: var(--sunken);
    color: var(--muted);
    border: 1px solid var(--line);
  }
  .label {
    white-space: nowrap;
  }
  .sep {
    width: 1px;
    background: var(--line);
    margin: 6px;
  }
  .search {
    margin-inline-start: auto;
    align-self: center;
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 10px;
    width: 230px;
    flex: none;
    color: var(--faint);
    background: var(--sunken);
    font-size: 13px;
  }
  .search:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .search input {
    border: 0;
    background: none;
    outline: none;
    min-width: 0;
    flex: 1;
    color: var(--ink);
    font: inherit;
  }
  kbd {
    font: 500 11px/1.4 inherit;
    direction: ltr;
    unicode-bidi: isolate;
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 4px;
    padding: 0 5px;
    color: var(--muted);
    background: var(--surface);
  }
</style>
