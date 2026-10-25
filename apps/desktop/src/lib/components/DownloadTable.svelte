<script lang="ts" module>
  /** The HTTP status a failed download's error names (`the server
   * answered 403 …`), if any. */
  export function errorCode(error: string | null): string | null {
    return /\banswered (\d{3})\b/.exec(error ?? "")?.[1] ?? null;
  }
</script>

<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { categoryIcon } from "./Tree.svelte";
  import { locale } from "../i18n.svelte";
  import { lastTry, left, percent, size, speed } from "../format";
  import { COLUMNS, clampWidth, load, move, save, toggle, visible, type ColumnId, type Columns } from "../columns";
  import type { DownloadView } from "../bindings/DownloadView";

  let {
    downloads,
    selected = $bindable(),
    label,
    chooser = $bindable(),
    onopen,
    oncontext,
  }: {
    downloads: DownloadView[];
    /** The selected downloads' ids, in table order. */
    selected: number[];
    /** What the table shows (the tree's choice), for screen readers. */
    label: string;
    /** The columns chooser is open (the «Columns» button, or View ›
     * Columns). */
    chooser: boolean;
    onopen: (id: number) => void;
    /** A right click (or the menu key) on a row, at that point. */
    oncontext: (x: number, y: number) => void;
  } = $props();

  // The columns as the user left them (M5.5).
  let columns = $state<Columns>(load());
  const shownColumns = $derived(visible(columns));
  function keep(next: Columns) {
    columns = next;
    save(next);
  }

  // Resizing: the grip at a header's end follows the pointer.
  function startResize(event: PointerEvent, id: ColumnId, width: number) {
    event.preventDefault();
    event.stopPropagation();
    const grip = event.currentTarget as HTMLElement;
    grip.setPointerCapture(event.pointerId);
    const startX = event.clientX;
    // In a right-to-left table the end is on the left: moving left widens.
    const sign = getComputedStyle(grip).direction === "rtl" ? -1 : 1;
    const moved = (e: PointerEvent) => {
      columns = { ...columns, widths: { ...columns.widths, [id]: clampWidth(width + sign * (e.clientX - startX)) } };
    };
    const done = () => {
      grip.removeEventListener("pointermove", moved);
      grip.removeEventListener("pointerup", done);
      grip.removeEventListener("lostpointercapture", done);
      save(columns);
    };
    grip.addEventListener("pointermove", moved);
    grip.addEventListener("pointerup", done);
    // A drag the system cancels ends too.
    grip.addEventListener("lostpointercapture", done);
  }
  // The chooser: its first box takes the focus when it opens; Escape, the
  // button or a click elsewhere closes it, and the focus goes back to the
  // button (review of #101).
  let colButton: HTMLButtonElement | undefined = $state();
  let chooserBox: HTMLDivElement | undefined = $state();
  $effect(() => {
    if (chooser) void tick().then(() => chooserBox?.querySelector<HTMLInputElement>("input:not(:disabled)")?.focus());
  });
  function closeChooser(refocus: boolean) {
    chooser = false;
    if (refocus) colButton?.focus();
  }
  // From the keyboard: the grip is a slider, the arrows change the width.
  function resizeKey(event: KeyboardEvent, id: ColumnId, width: number) {
    const step = event.shiftKey ? 32 : 8;
    const wider = getComputedStyle(event.currentTarget as HTMLElement).direction === "rtl" ? "ArrowLeft" : "ArrowRight";
    const narrower = wider === "ArrowLeft" ? "ArrowRight" : "ArrowLeft";
    if (event.key !== wider && event.key !== narrower) return;
    event.preventDefault();
    keep({ ...columns, widths: { ...columns.widths, [id]: clampWidth(width + (event.key === wider ? step : -step)) } });
  }

  // Reordering: a header dragged onto another goes before it.
  let dragging = $state<ColumnId | null>(null);
  function drop(before: ColumnId) {
    if (dragging && dragging !== before) keep(move(columns, dragging, before));
    dragging = null;
  }

  function context(event: MouseEvent, index: number) {
    event.preventDefault();
    const id = downloads[index]!.id;
    // A right click on a row that is not selected selects it alone.
    if (!selected.includes(id)) {
      selected = [id];
      anchor = id;
    }
    focus = id;
    oncontext(event.clientX, event.clientY);
  }

  const fraction = (d: DownloadView) => (d.size ? d.done / d.size : d.status === "done" ? 1 : 0);
  const pct = (d: DownloadView) => percent(locale.lang, locale.digits, fraction(d));

  // The row the keyboard is on, and where a Shift range starts.
  let focus = $state<number | null>(null);
  let anchor = $state<number | null>(null);
  const focusIndex = $derived(Math.max(downloads.findIndex((d) => d.id === (focus ?? selected[0])), 0));

  function range(from: number, to: number): number[] {
    const [a, b] = from <= to ? [from, to] : [to, from];
    return downloads.slice(a, b + 1).map((d) => d.id);
  }

  /** A click, as file lists do: alone it selects that row; with Ctrl (⌘ on
   * macOS) it adds or removes it; with Shift it selects from the anchor. */
  function pick(event: MouseEvent, index: number) {
    const id = downloads[index]!.id;
    if (event.shiftKey && anchor !== null) {
      const from = downloads.findIndex((d) => d.id === anchor);
      selected = range(from < 0 ? index : from, index);
    } else if (event.ctrlKey || event.metaKey) {
      const set = new Set(selected);
      if (!set.delete(id)) set.add(id);
      selected = downloads.filter((d) => set.has(d.id)).map((d) => d.id);
      anchor = id;
    } else {
      selected = [id];
      anchor = id;
    }
    focus = id;
  }

  // The arrows, Home and End move (Shift extends the selection); Space
  // selects the row; Enter opens it; the menu key (or Shift+F10) opens the
  // right-click menu at the row. Select all is the window's (⌘A, Ctrl+A).
  function onkeydown(event: KeyboardEvent, index: number) {
    if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
      event.preventDefault();
      const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
      if (!selected.includes(downloads[index]!.id)) selected = [downloads[index]!.id];
      oncontext(box.left + 24, box.bottom - 4);
      return;
    }
    const next =
      event.key === "ArrowDown" ? index + 1
      : event.key === "ArrowUp" ? index - 1
      : event.key === "Home" ? 0
      : event.key === "End" ? downloads.length - 1
      : event.key === " " ? index
      : -1;
    if (next >= 0 && next < downloads.length) {
      event.preventDefault();
      const id = downloads[next]!.id;
      const from = anchor === null ? -1 : downloads.findIndex((d) => d.id === anchor);
      // A Shift range from an anchor that is gone (filtered, deleted)
      // starts here (review of #96: it cleared the selection).
      if (event.shiftKey && from >= 0) {
        selected = range(from, next);
      } else {
        selected = [id];
        anchor = id;
      }
      focus = id;
      document.getElementById(`row-${id}`)?.focus();
    } else if (event.key === "Enter" && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
      // Enter opens it here; the window does not see it again.
      event.preventDefault();
      event.stopPropagation();
      const id = downloads[index]?.id;
      if (id !== undefined) onopen(id);
    }
  }
</script>

<svelte:document onpointerdown={(e) => chooser && !chooserBox?.contains(e.target as Node) && !colButton?.contains(e.target as Node) && closeChooser(false)} />

{#snippet cell(d: DownloadView, id: ColumnId)}
  {#if id === "name"}
    <td>
      <span class="nm">
        <span class="ft" aria-hidden="true"><Icon name={categoryIcon(d.category)} size={14} /></span>
        <FileName name={d.name} />
      </span>
    </td>
  {:else if id === "size"}
    <td class="num muted">{d.size === null ? "" : size(locale.lang, locale.digits, d.size)}</td>
  {:else if id === "status"}
    <td>
      {#if d.needs_ok}
        <span class="st paused"><Icon name="circle-alert" size={14} />{locale.t("status-needs-ok")}</span>
      {:else if d.waits_for_browser}
        <span class="st paused"><Icon name="link-2" size={14} />{locale.t("status-open-browser")}</span>
      {:else if d.status === "active"}
        <span class="st">
          <span class="mini" aria-hidden="true"><b style:inline-size="{Math.round(fraction(d) * 100)}%"></b></span>
          <span class="pct num">{pct(d)}</span>
        </span>
      {:else if d.status === "paused"}
        <span class="st paused"><Icon name="pause" size={14} />{locale.t("st-paused")} <span class="num">{pct(d)}</span></span>
      {:else if d.status === "done"}
        <span class="st done"><Icon name="circle-check" size={14} />{locale.t("status-done")}</span>
      {:else if d.status === "failed"}
        {@const code = errorCode(d.error)}
        <span class="st failed" title={d.error ?? undefined}><Icon name="circle-alert" size={14} />{code ? locale.t("st-error", { code }) : locale.t("st-failed")}</span>
      {:else}
        <span class="st waiting"><Icon name="list-end" size={14} />{locale.t("st-in-queue")}</span>
      {/if}
    </td>
  {:else if id === "left"}
    <td class="num muted">{d.status === "active" && d.size !== null ? left(locale.lang, locale.digits, d.size - d.done, d.speed) : ""}</td>
  {:else if id === "speed"}
    <td class="num">{d.status === "active" ? speed(locale.lang, locale.digits, d.speed) : ""}</td>
  {:else if id === "queue"}
    <td class="muted"><bdi>{d.queue === "main" ? locale.t("queue-main") : d.queue}</bdi></td>
  {:else if id === "last"}
    <td class="num muted">{d.last_try ? lastTry(locale.lang, locale.digits, d.last_try, locale.t) : ""}</td>
  {:else}
    <td class="muted"><span class="desc" title={d.description ?? undefined}>{d.description ?? ""}</span></td>
  {/if}
{/snippet}

<div class="scroll">
  <table class="grid" aria-label={label} aria-multiselectable="true" role="grid">
    <colgroup>
      {#each shownColumns as c (c.id)}<col style:width={c.width === null ? undefined : `${c.width}px`} />{/each}
      <col class="c-cols" />
    </colgroup>
    <thead>
      <tr>
        {#each shownColumns as c (c.id)}
          <th
            scope="col"
            draggable="true"
            class:target={dragging !== null && dragging !== c.id}
            ondragstart={(e) => {
              dragging = c.id;
              e.dataTransfer?.setData("text/plain", c.id);
            }}
            ondragover={(e) => dragging && e.preventDefault()}
            ondrop={(e) => {
              e.preventDefault();
              drop(c.id);
            }}
            ondragend={() => (dragging = null)}
          >
            {locale.t(c.key)}
            {#if c.width !== null}
              <!-- A focusable separator is a splitter, a widget (WAI-ARIA). -->
              <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
              <span
                class="grip"
                role="separator"
                aria-orientation="vertical"
                aria-label={locale.t("col-resize", { column: locale.t(c.key) })}
                aria-valuenow={c.width}
                aria-valuemin={56}
                aria-valuemax={600}
                tabindex="0"
                draggable="false"
                onpointerdown={(e) => startResize(e, c.id, c.width ?? 100)}
                onkeydown={(e) => resizeKey(e, c.id, c.width ?? 100)}
              ></span>
            {/if}
          </th>
        {/each}
        <th class="cols" scope="col">
          <button bind:this={colButton} class="colbtn" aria-label={locale.t("cmd-columns")} title={locale.t("cmd-columns")} aria-expanded={chooser} onclick={() => (chooser ? closeChooser(true) : (chooser = true))}>
            <Icon name="columns-3" size={15} />
          </button>
          {#if chooser}
            <!-- Escape from any of its boxes closes it. -->
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <div
              class="chooser"
              role="group"
              aria-label={locale.t("cmd-columns")}
              bind:this={chooserBox}
              onkeydown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  closeChooser(true);
                }
              }}
            >
              {#each COLUMNS as c (c.id)}
                <label class="choice">
                  <input type="checkbox" checked={!columns.hidden.includes(c.id)} disabled={c.id === "name"} onchange={() => keep(toggle(columns, c.id))} />
                  {locale.t(c.key)}
                </label>
              {/each}
            </div>
          {/if}
        </th>
      </tr>
    </thead>
    <tbody>
      {#each downloads as d, i (d.id)}
        <tr
          id="row-{d.id}"
          tabindex={i === focusIndex ? 0 : -1}
          aria-selected={selected.includes(d.id)}
          onclick={(e) => pick(e, i)}
          ondblclick={() => onopen(d.id)}
          oncontextmenu={(e) => context(e, i)}
          onkeydown={(e) => onkeydown(e, i)}
        >
          {#each shownColumns as c (c.id)}{@render cell(d, c.id)}{/each}
          <td class="cols"></td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .desc {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .scroll {
    overflow: auto;
    flex: 1;
    min-height: 0;
  }
  table.grid {
    width: 100%;
    min-width: 640px;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 13px;
  }
  .c-cols {
    width: 34px;
  }
  th {
    text-align: start;
    font-size: 12px;
    font-weight: 600;
    color: var(--faint);
    padding: 7px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    position: sticky;
    top: 0;
    white-space: nowrap;
  }
  th[draggable="true"] {
    cursor: grab;
  }
  th.target {
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .grip {
    position: absolute;
    inset-block: 4px;
    inset-inline-end: -4px;
    width: 8px;
    cursor: col-resize;
    z-index: 2;
  }
  .grip:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    border-radius: 3px;
  }
  th.cols,
  td.cols {
    width: 34px;
    padding: 0;
    text-align: center;
  }
  .colbtn {
    border: 0;
    background: none;
    color: var(--muted);
    padding: 4px;
    border-radius: 6px;
    cursor: pointer;
    display: inline-grid;
  }
  .colbtn:hover {
    background: var(--sunken);
  }
  .chooser {
    position: absolute;
    top: 100%;
    inset-inline-end: 4px;
    z-index: 5;
    min-width: 190px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-weight: 400;
    color: var(--ink);
    text-align: start;
  }
  .choice {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: 6px;
  }
  .choice:hover {
    background: var(--sunken);
  }
  th + th::before {
    content: "";
    position: absolute;
    inset-block: 7px;
    inset-inline-start: 0;
    width: 1px;
    background: var(--line);
  }
  td {
    padding: 6px 10px;
    border-bottom: 1px solid color-mix(in oklab, var(--line) 60%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    vertical-align: middle;
  }
  tr {
    cursor: default;
  }
  tr[aria-selected="true"] td {
    background: var(--sel);
  }
  tr:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .nm {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .ft {
    width: 24px;
    height: 24px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    flex: none;
    background: var(--sunken);
    color: var(--muted);
    border: 1px solid var(--line);
  }
  .muted {
    color: var(--muted);
  }
  .st {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .st :global(.icon) {
    flex: none;
  }
  .pct {
    min-width: 34px;
    font-size: 12px;
  }
  .mini {
    display: inline-block;
    width: 64px;
    height: 6px;
    border-radius: 99px;
    background: var(--feather-bg);
    overflow: hidden;
    position: relative;
  }
  .mini b {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    border-radius: inherit;
    background: var(--feather);
  }
  .done {
    color: var(--green);
  }
  .paused {
    color: var(--amber);
  }
  .failed {
    color: var(--red);
  }
  .waiting {
    color: var(--muted);
  }
</style>
