<script lang="ts" module>
  /** The HTTP status a failed download's error names (`the server
   * answered 403 …`), if any. */
  export function errorCode(error: string | null): string | null {
    return /\banswered (\d{3})\b/.exec(error ?? "")?.[1] ?? null;
  }
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { categoryIcon } from "./Tree.svelte";
  import { locale } from "../i18n.svelte";
  import { isMod } from "../keys";
  import { left, percent, size, speed } from "../format";
  import type { DownloadView } from "../bindings/DownloadView";

  let {
    downloads,
    selected = $bindable(),
    onopen,
  }: {
    downloads: DownloadView[];
    /** The selected downloads' ids, in table order. */
    selected: number[];
    onopen: (id: number) => void;
  } = $props();

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
  // selects the row; Enter opens it; Ctrl+A (⌘A) selects every row.
  function onkeydown(event: KeyboardEvent, index: number) {
    if (isMod(event, "a")) {
      event.preventDefault();
      selected = downloads.map((d) => d.id);
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
      if (event.shiftKey && anchor !== null) {
        selected = range(downloads.findIndex((d) => d.id === anchor), next);
      } else {
        selected = [id];
        anchor = id;
      }
      focus = id;
      document.getElementById(`row-${id}`)?.focus();
    } else if (event.key === "Enter") {
      const id = downloads[index]?.id;
      if (id !== undefined) onopen(id);
    }
  }
</script>

<div class="scroll">
  <table class="grid" aria-label={locale.t("nav-all")} aria-multiselectable="true" role="grid">
    <colgroup>
      <col class="c-name" /><col class="c-size" /><col class="c-status" /><col class="c-left" /><col class="c-speed" /><col class="c-last" /><col class="c-desc" />
    </colgroup>
    <thead>
      <tr>
        <th scope="col">{locale.t("col-name")}</th>
        <th scope="col">{locale.t("col-size")}</th>
        <th scope="col">{locale.t("col-status")}</th>
        <th scope="col">{locale.t("col-left")}</th>
        <th scope="col">{locale.t("col-speed")}</th>
        <th scope="col">{locale.t("col-last-try")}</th>
        <th scope="col">{locale.t("col-description")}</th>
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
          onkeydown={(e) => onkeydown(e, i)}
        >
          <td>
            <span class="nm">
              <span class="ft" aria-hidden="true"><Icon name={categoryIcon(d.category)} size={14} /></span>
              <FileName name={d.name} />
            </span>
          </td>
          <td class="num muted">{d.size === null ? "" : size(locale.lang, locale.digits, d.size)}</td>
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
          <td class="num muted">{d.status === "active" && d.size !== null ? left(locale.lang, locale.digits, d.size - d.done, d.speed) : ""}</td>
          <td class="num">{d.status === "active" ? speed(locale.lang, locale.digits, d.speed) : ""}</td>
          <!-- «Last try» and «Description» come with #95 (zajil-core). -->
          <td class="num faint"></td>
          <td class="faint"></td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .scroll {
    overflow: auto;
    flex: 1;
    min-height: 0;
  }
  table.grid {
    width: 100%;
    min-width: 900px;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 13px;
  }
  .c-name {
    width: 34%;
  }
  .c-size {
    width: 11%;
  }
  .c-status {
    width: 15%;
  }
  .c-left {
    width: 10%;
  }
  .c-speed {
    width: 11%;
  }
  .c-last {
    width: 10%;
  }
  .c-desc {
    width: 9%;
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
  .faint {
    color: var(--faint);
  }
  .st {
    display: inline-flex;
    align-items: center;
    gap: 6px;
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
