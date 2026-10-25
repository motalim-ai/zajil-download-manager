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
  import { tick } from "svelte";
  import { shortcut } from "../keys";
  import { DEFAULT, load, save, shift, shown, toggle, type ToolbarPrefs } from "../toolbar";

  let {
    search = $bindable(),
    can,
    hint,
    onaction,
  }: {
    search: string;
    /** A button's longer name, if it has one (what it acts on). */
    hint: (action: ToolAction) => string | undefined;
    /** Whether a button has something to act on now. */
    can: (action: ToolAction) => boolean;
    onaction: (action: ToolAction) => void;
  } = $props();

  // A colour for each kind of action (owner's decision on #83): green to
  // run, amber to stop, red to delete, grey for settings and the schedule,
  // blue to add a link. A line between the groups (`toolbar.ts`).
  const LOOK: Record<ToolAction, { icon: string; key: string; kind: string }> = {
    add: { icon: "link-2", key: "tb-add", kind: "add" },
    resume: { icon: "play", key: "tb-resume", kind: "go" },
    pause: { icon: "pause", key: "tb-pause", kind: "stop" },
    "pause-all": { icon: "octagon-pause", key: "tb-pause-all", kind: "stop" },
    delete: { icon: "trash-2", key: "tb-delete", kind: "del" },
    "delete-done": { icon: "list-x", key: "tb-delete-done", kind: "del" },
    settings: { icon: "settings", key: "tb-settings", kind: "calm" },
    schedule: { icon: "calendar-clock", key: "tb-schedule", kind: "calm" },
    "run-queue": { icon: "list-start", key: "tb-run-queue", kind: "go" },
    "halt-queue": { icon: "list-end", key: "tb-halt-queue", kind: "stop" },
  };

  // As the user leaves it (an IDM touch): big or small buttons, which,
  // and in what order; a right-click on the toolbar changes it.
  let prefs = $state<ToolbarPrefs>(load());
  const buttons = $derived(shown(prefs));
  function keep(next: ToolbarPrefs) {
    prefs = next;
    save(next);
  }
  let menu = $state<{ x: number; y: number } | null>(null);
  let customizing = $state(false);
  let panel: HTMLDivElement | undefined = $state();
  function context(event: MouseEvent) {
    if ((event.target as Element).closest(".search")) return;
    event.preventDefault();
    menu = { x: event.clientX, y: event.clientY };
    void tick().then(() => document.querySelector<HTMLButtonElement>(".tbmenu [role^='menuitem']")?.focus());
  }
  function onMenuKey(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      menu = null;
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const items = [...document.querySelectorAll<HTMLButtonElement>(".tbmenu [role^='menuitem']")];
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      items[(at + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
    }
  }
  async function customize() {
    menu = null;
    customizing = true;
    await tick();
    panel?.querySelector<HTMLInputElement>("input")?.focus();
  }
</script>

<svelte:document
  onpointerdown={(e) => {
    const at = e.target as Element;
    if (menu && !at.closest?.(".tbmenu")) menu = null;
    if (customizing && !at.closest?.(".tbpanel")) customizing = false;
  }}
/>

<!-- A right-click on the toolbar: big or small buttons, and customizing. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="toolbar" class:small={prefs.small} role="toolbar" tabindex="-1" aria-label={locale.t("tb-toolbar")} oncontextmenu={context}>
  {#each buttons as b (b.action)}
    {@const look = LOOK[b.action]}
    {#if b.line}<span class="sep" aria-hidden="true"></span>{/if}
    <button
      id="tb-{b.action}"
      class="tb {look.kind}"
      title={hint(b.action) ?? (prefs.small ? locale.t(look.key) : undefined)}
      aria-label={prefs.small ? locale.t(look.key) : undefined}
      disabled={!can(b.action)}
      onclick={() => onaction(b.action)}
    >
      <span class="tile"><Icon name={look.icon} size={prefs.small ? 17 : 22} stroke={1.7} /></span>
      {#if !prefs.small}<span class="label">{locale.t(look.key)}</span>{/if}
    </button>
  {/each}
  <label class="search">
    <Icon name="search" size={16} />
    <input id="search" type="search" bind:value={search} placeholder={locale.t("search-placeholder")} aria-label={locale.t("search-placeholder")} />
    <kbd aria-hidden="true">{shortcut("mod+F")}</kbd>
  </label>
</div>

{#if menu}
  <div class="tbmenu" role="menu" tabindex="-1" aria-label={locale.t("tb-toolbar")} style:left="{menu.x}px" style:top="{menu.y}px" onkeydown={onMenuKey}>
    <button class="it" role="menuitemradio" aria-checked={!prefs.small} onclick={() => ((menu = null), keep({ ...prefs, small: false }))}>
      <span class="mark">{#if !prefs.small}<Icon name="check" size={14} />{/if}</span>{locale.t("tb-big")}
    </button>
    <button class="it" role="menuitemradio" aria-checked={prefs.small} onclick={() => ((menu = null), keep({ ...prefs, small: true }))}>
      <span class="mark">{#if prefs.small}<Icon name="check" size={14} />{/if}</span>{locale.t("tb-small")}
    </button>
    <hr />
    <button class="it" role="menuitem" onclick={() => void customize()}><span class="mark"></span>{locale.t("tb-customize")}</button>
  </div>
{/if}

{#if customizing}
  <div class="tbpanel" role="dialog" aria-label={locale.t("tb-customize-title")} bind:this={panel} tabindex="-1" onkeydown={(e) => e.key === "Escape" && (customizing = false)}>
    <h2>{locale.t("tb-customize-title")}</h2>
    <ol>
      {#each prefs.order as action, i (action)}
        {@const look = LOOK[action]}
        <li>
          <label><input type="checkbox" checked={!prefs.hidden.includes(action)} onchange={() => keep(toggle(prefs, action))} />
            <Icon name={look.icon} size={16} />{locale.t(look.key)}</label>
          <span class="moves">
            <button class="btn quiet" aria-label={locale.t("tb-earlier", { name: locale.t(look.key) })} disabled={i === 0} onclick={() => keep(shift(prefs, action, -1))}><Icon name="arrow-up" size={14} /></button>
            <button class="btn quiet" aria-label={locale.t("tb-later", { name: locale.t(look.key) })} disabled={i === prefs.order.length - 1} onclick={() => keep(shift(prefs, action, 1))}><Icon name="arrow-down" size={14} /></button>
          </span>
        </li>
      {/each}
    </ol>
    <div class="actions">
      <button class="btn" onclick={() => keep({ ...DEFAULT, order: [...DEFAULT.order] })}>{locale.t("tb-reset")}</button>
      <button class="btn primary" onclick={() => (customizing = false)}>{locale.t("tb-done")}</button>
    </div>
  </div>
{/if}

<style>
  .small .tb {
    min-width: 34px;
    padding: 4px;
  }
  .small .tile {
    width: 28px;
    height: 28px;
    border-radius: 8px;
  }
  .tbmenu {
    position: fixed;
    z-index: 30;
    min-width: 200px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
    font-size: 13px;
  }
  .tbmenu .it {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border: 0;
    background: none;
    border-radius: 6px;
    text-align: start;
    font: inherit;
    color: var(--ink);
  }
  .tbmenu .it:hover,
  .tbmenu .it:focus-visible {
    background: var(--sel);
    outline: none;
  }
  .tbmenu .mark {
    width: 14px;
    display: inline-grid;
  }
  .tbmenu hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 4px 6px;
  }
  .tbpanel {
    position: fixed;
    z-index: 30;
    top: 110px;
    inset-inline-start: 24px;
    width: 300px;
    /* Never past the window's bottom (its smallest is 560 px high). */
    max-height: calc(100vh - 130px);
    overflow: auto;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 12px;
    box-shadow: var(--shadow);
    padding: 14px 16px;
  }
  .tbpanel h2 {
    margin: 0 0 10px;
    font-size: 14px;
  }
  .tbpanel ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tbpanel li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 13px;
  }
  .tbpanel label {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .tbpanel .moves .btn {
    padding: 2px 6px;
  }
  .tbpanel .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }

  .toolbar {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    overflow: hidden;
  }
  .tb {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    min-width: 62px;
    flex: none;
    padding: 6px 4px 4px;
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
    flex: none;
    background: var(--line);
    margin: 6px 4px;
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
    flex: 0 1 230px;
    min-width: 150px;
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
