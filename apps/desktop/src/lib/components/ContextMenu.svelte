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
    /** `refocus`: the focus goes back to the table (Escape, Tab, a
     * command), not after a click elsewhere. */
    onclose: (refocus: boolean) => void;
  } = $props();

  // The icon of each command, as in the design.
  const ICONS: Partial<Record<Command, string>> = {
    open: "t-file",
    "open-folder": "folder-open",
    resume: "play",
    pause: "pause",
    progress: "c-window",
    "copy-link": "c-copy",
    "refresh-link": "c-refresh",
    delete: "trash-2",
    properties: "info",
    "remove-from-queue": "list-x",
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
    onclose(true);
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
      if (sub && event.key === "Escape") return closeSub();
      onclose(true);
    }
  }

  // «Move to queue» opens with the arrow toward its submenu, Enter or
  // Space; the other arrow or Escape goes back (review of #101).
  let subButton: HTMLButtonElement | undefined = $state();
  const inward = () => (locale.dir === "rtl" ? "ArrowLeft" : "ArrowRight");
  async function openSub() {
    sub = true;
    await tick();
    menu?.querySelector<HTMLButtonElement>('.sub [role="menuitem"]')?.focus();
  }
  function closeSub() {
    sub = false;
    subButton?.focus();
  }
  function onSubKey(event: KeyboardEvent) {
    if (event.key === inward() || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      event.stopPropagation();
      void openSub();
    }
  }
  function onInSubKey(event: KeyboardEvent) {
    if (event.key === (locale.dir === "rtl" ? "ArrowRight" : "ArrowLeft")) {
      event.preventDefault();
      event.stopPropagation();
      closeSub();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      event.stopPropagation();
      const items = [...(menu?.querySelectorAll<HTMLButtonElement>('.sub [role="menuitem"]') ?? [])];
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      items[(at + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
    }
  }
</script>

<svelte:document onpointerdown={(e) => !menu?.contains(e.target as Node) && onclose(false)} />

<div
  class="ctx"
  role="menu"
  aria-label={locale.t("row-menu")}
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
        <button
          bind:this={subButton}
          class="it"
          role="menuitem"
          aria-haspopup="menu"
          aria-expanded={sub}
          tabindex="-1"
          onclick={() => (sub ? closeSub() : void openSub())}
          onpointerenter={() => (sub = true)}
          onkeydown={onSubKey}
        >
          <Icon name="list-ordered" size={15} /><span>{locale.t("cmd-move-to-queue")}</span>
          <span class="more flip"><Icon name="chevron-right" size={14} /></span>
        </button>
        {#if sub}
          <div class="sub" role="menu" tabindex="-1" aria-label={locale.t("cmd-move-to-queue")} onkeydown={onInSubKey}>
            {#each queues as q (q.name)}
              <button class="it" role="menuitem" tabindex="-1" onclick={() => (onclose(true), onmove(q.name))}><bdi>{q.label}</bdi></button>
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
