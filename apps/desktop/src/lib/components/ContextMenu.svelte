<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import { shortcutOf, type Command } from "../commands";
  import { ROW_MENU, commandKey } from "../menus";

  let {
    x,
    y,
    queues,
    can,
    onrun,
    onmove,
    onclose,
  }: {
    /** Where the pointer was, in the window. */
    x: number;
    y: number;
    /** The queues «Move to queue» offers, with their shown names. */
    queues: { name: string; label: string }[];
    can: (command: Command) => boolean;
    onrun: (command: Command) => void;
    onmove: (queue: string) => void;
    onclose: () => void;
  } = $props();

  // The icon of each command, as in the design.
  const ICONS: Partial<Record<Command, string>> = {
    open: "file",
    "open-folder": "folder-open",
    resume: "play",
    pause: "pause",
    progress: "external-link",
    "copy-link": "copy",
    "refresh-link": "rotate-ccw",
    delete: "trash-2",
    properties: "info",
  };

  let menu: HTMLDivElement | undefined = $state();
  let sub = $state(false);
  // Kept inside the window.
  let place = $state({ left: 0, top: 0 });
  $effect(() => {
    void tick().then(() => {
      if (!menu) return;
      const box = menu.getBoundingClientRect();
      place = {
        left: Math.max(4, Math.min(x, window.innerWidth - box.width - 4)),
        top: Math.max(4, Math.min(y, window.innerHeight - box.height - 4)),
      };
      menu.querySelector<HTMLButtonElement>('[role="menuitem"]:not([aria-disabled="true"])')?.focus();
    });
  });

  function run(command: Command) {
    if (!can(command)) return;
    onclose();
    onrun(command);
  }

  function onkeydown(event: KeyboardEvent) {
    const items = [...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([aria-disabled="true"])') ?? [])];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      items[(at + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
    } else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      onclose();
    }
  }
</script>

<svelte:document onpointerdown={(e) => !menu?.contains(e.target as Node) && onclose()} />

<div
  class="ctx"
  role="menu"
  aria-label={locale.t("cmd-properties")}
  tabindex="-1"
  bind:this={menu}
  style:left="{place.left}px"
  style:top="{place.top}px"
  {onkeydown}
>
  {#each ROW_MENU as item, i (i)}
    {#if item === "-"}
      <hr />
    {:else if "queues" in item}
      <div class="slot">
        <button class="it" role="menuitem" aria-haspopup="menu" aria-expanded={sub} tabindex="-1" onclick={() => (sub = !sub)} onpointerenter={() => (sub = true)}>
          <Icon name="list-ordered" size={15} /><span>{locale.t("cmd-move-to-queue")}</span>
          <span class="more flip"><Icon name="chevron-right" size={14} /></span>
        </button>
        {#if sub}
          <div class="sub" role="menu" aria-label={locale.t("cmd-move-to-queue")}>
            {#each queues as q (q.name)}
              <button class="it" role="menuitem" tabindex="-1" onclick={() => (onclose(), onmove(q.name))}><bdi>{q.label}</bdi></button>
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      {@const off = !can(item.command)}
      <button class="it" class:bad={item.danger} role="menuitem" aria-disabled={off} tabindex="-1" onclick={() => run(item.command)} onpointerenter={() => (sub = false)}>
        <Icon name={ICONS[item.command] ?? "file"} size={15} /><span>{locale.t(commandKey(item.command))}</span>
        {#if shortcutOf(item.command)}<kbd>{shortcutOf(item.command)}</kbd>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .ctx {
    position: fixed;
    z-index: 30;
    width: 268px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    font-size: 13px;
    display: flex;
    flex-direction: column;
  }
  .slot {
    position: relative;
    display: flex;
    flex-direction: column;
  }
  .it {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 10px;
    border-radius: 6px;
    border: 0;
    background: none;
    color: var(--ink);
    text-align: start;
    font: inherit;
    cursor: default;
  }
  .it :global(.icon) {
    color: var(--muted);
  }
  .it:hover:not([aria-disabled="true"]),
  .it:focus-visible {
    background: var(--sel);
    outline: none;
  }
  .it[aria-disabled="true"] {
    color: var(--faint);
  }
  .bad,
  .bad :global(.icon) {
    color: var(--red);
  }
  kbd,
  .more {
    margin-inline-start: auto;
  }
  kbd {
    font: 500 11px/1.4 inherit;
    direction: ltr;
    unicode-bidi: isolate;
    color: var(--muted);
  }
  :global([dir="rtl"]) .flip {
    display: inline-flex;
    transform: scaleX(-1);
  }
  .sub {
    position: absolute;
    top: -4px;
    inset-inline-start: calc(100% + 4px);
    min-width: 180px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 4px 6px;
  }
</style>
