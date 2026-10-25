<script lang="ts">
  import { tick } from "svelte";
  import { locale } from "../i18n.svelte";
  import { shortcutOf, type Command } from "../commands";
  import { MENU_BAR, accessLetter, commandKey } from "../menus";

  let {
    can,
    onrun,
  }: {
    /** Whether a command has something to act on now. */
    can: (command: Command) => boolean;
    onrun: (command: Command) => void;
  } = $props();

  // The open menu, and its item with the focus.
  let open = $state<number | null>(null);
  let bar: HTMLDivElement | undefined = $state();

  async function show(index: number, focusItem: "first" | "none" = "first") {
    open = index;
    await tick();
    if (focusItem === "first") bar?.querySelector<HTMLButtonElement>(`#menu-${index} [role="menuitem"]:not([aria-disabled="true"])`)?.focus();
  }
  function close(refocus = true) {
    const was = open;
    open = null;
    if (refocus && was !== null) document.getElementById(`menubar-${was}`)?.focus();
  }
  function run(command: Command) {
    if (!can(command)) return;
    close(false);
    onrun(command);
  }

  // Alt with a menu's underlined letter opens it (the next one, if two share
  // it, as «م» does in Arabic); Alt alone, then its release, goes to the bar.
  let altAlone = false;
  function onWindowKeydown(event: KeyboardEvent) {
    altAlone = event.key === "Alt";
    if (!event.altKey || event.ctrlKey || event.metaKey || event.key === "Alt") return;
    const letter = event.key.toLocaleLowerCase();
    const names = MENU_BAR.map((m) => accessLetter(locale.t(m.key)).toLocaleLowerCase());
    const start = open === null ? 0 : open + 1;
    const order = [...names.keys()].map((i) => (start + i) % names.length);
    const found = order.find((i) => names[i] === letter);
    if (found !== undefined) {
      event.preventDefault();
      void show(found);
    }
  }
  function onWindowKeyup(event: KeyboardEvent) {
    if (event.key === "Alt" && altAlone) {
      altAlone = false;
      document.getElementById("menubar-0")?.focus();
    }
  }

  function onBarKey(event: KeyboardEvent, index: number) {
    // Right and left follow the reading direction.
    const forward = locale.dir === "rtl" ? "ArrowLeft" : "ArrowRight";
    const back = locale.dir === "rtl" ? "ArrowRight" : "ArrowLeft";
    const count = MENU_BAR.length;
    if (event.key === forward || event.key === back) {
      event.preventDefault();
      const next = (index + (event.key === forward ? 1 : -1) + count) % count;
      if (open !== null) void show(next);
      else document.getElementById(`menubar-${next}`)?.focus();
    } else if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void show(index);
    } else if (event.key === "Escape") {
      close();
    }
  }

  function onMenuKey(event: KeyboardEvent, index: number) {
    const items = [...(bar?.querySelectorAll<HTMLButtonElement>(`#menu-${index} [role="menuitem"]:not([aria-disabled="true"])`) ?? [])];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    const forward = locale.dir === "rtl" ? "ArrowLeft" : "ArrowRight";
    const back = locale.dir === "rtl" ? "ArrowRight" : "ArrowLeft";
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const next = (at + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
      items[next]?.focus();
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      items[event.key === "Home" ? 0 : items.length - 1]?.focus();
    } else if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === forward || event.key === back) {
      event.preventDefault();
      const count = MENU_BAR.length;
      void show((index + (event.key === forward ? 1 : -1) + count) % count);
    } else if (event.key === "Tab") {
      close(false);
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} onkeyup={onWindowKeyup} />
<svelte:document onpointerdown={(e) => open !== null && !bar?.contains(e.target as Node) && close(false)} />

<div class="menubar" role="menubar" aria-label={locale.t("menu-bar")} bind:this={bar}>
  {#each MENU_BAR as menu, i (menu.key)}
    {@const name = locale.t(menu.key)}
    {@const first = accessLetter(name)}
    <div class="slot">
      <button
        id="menubar-{i}"
        class="top"
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={open === i}
        tabindex={i === 0 ? 0 : -1}
        onclick={() => (open === i ? close() : void show(i, "none"))}
        onpointerenter={() => open !== null && open !== i && void show(i, "none")}
        onkeydown={(e) => onBarKey(e, i)}
      ><u>{first}</u>{name.slice(first.length)}</button>
      {#if open === i}
        <div id="menu-{i}" class="menu" role="menu" aria-labelledby="menubar-{i}" tabindex="-1" onkeydown={(e) => onMenuKey(e, i)}>
          {#each menu.items as item, j (j)}
            {#if item === "-"}
              <hr />
            {:else if "command" in item}
              {@const off = !can(item.command)}
              <button class="item" class:danger={item.danger} role="menuitem" aria-disabled={off} tabindex="-1" onclick={() => run(item.command)}>
                <span>{locale.t(commandKey(item.command))}</span>
                {#if shortcutOf(item.command)}<kbd>{shortcutOf(item.command)}</kbd>{/if}
              </button>
            {/if}
          {/each}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .menubar {
    display: flex;
    gap: 2px;
    padding: 2px 8px;
    font-size: 13px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    position: relative;
    z-index: 20;
  }
  .slot {
    position: relative;
  }
  .top {
    padding: 3px 10px;
    border-radius: 6px;
    border: 0;
    background: none;
    color: var(--ink);
    cursor: default;
    font: inherit;
  }
  .top:hover,
  .top[aria-expanded="true"] {
    background: var(--sel);
  }
  u {
    text-decoration-thickness: 1px;
    text-underline-offset: 3px;
  }
  .menu {
    position: absolute;
    top: 100%;
    inset-inline-start: 0;
    min-width: 248px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 5px 10px;
    border-radius: 6px;
    border: 0;
    background: none;
    color: var(--ink);
    text-align: start;
    font: inherit;
    cursor: default;
  }
  .item:hover:not([aria-disabled="true"]),
  .item:focus-visible {
    background: var(--sel);
    outline: none;
  }
  .item[aria-disabled="true"] {
    color: var(--faint);
  }
  .danger {
    color: var(--red);
  }
  kbd {
    margin-inline-start: auto;
    font: 500 11px/1.4 inherit;
    direction: ltr;
    unicode-bidi: isolate;
    color: var(--muted);
  }
  hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 4px 6px;
    width: auto;
  }
</style>
