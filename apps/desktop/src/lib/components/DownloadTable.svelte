<script lang="ts">
  import FileName from "./FileName.svelte";
  import Feathers from "./Feathers.svelte";
  import StatusChip from "./StatusChip.svelte";
  import { locale } from "../i18n.svelte";
  import { left, percent, size, speed } from "../format";
  import type { DownloadView } from "../bindings/DownloadView";

  let {
    downloads,
    selected = $bindable(),
    onopen,
  }: { downloads: DownloadView[]; selected: number | null; onopen: (id: number) => void } = $props();

  const extension = (name: string) => (/\.([A-Za-z0-9]{1,4})[A-Za-z0-9]*$/.exec(name)?.[1] ?? "").toUpperCase();
  const fraction = (d: DownloadView) => (d.size ? d.done / d.size : d.status === "done" ? 1 : 0);

  // The row Tab reaches: the selected one, else the first.
  const current = $derived(Math.max(downloads.findIndex((d) => d.id === selected), 0));

  // Keyboard first: the arrows, Home and End move the selection; Space
  // selects the focused row; Enter (or a double click) opens its details.
  // An arrow on a row that is not
  // selected selects that row first (review of #46: it skipped it).
  function onkeydown(event: KeyboardEvent, index: number) {
    const here = downloads[index]?.id !== selected;
    const next =
      event.key === "ArrowDown" ? (here ? index : index + 1)
      : event.key === "ArrowUp" ? (here ? index : index - 1)
      : event.key === "Home" ? 0
      : event.key === "End" ? downloads.length - 1
      : event.key === " " ? index
      : -1;
    if (next >= 0 && next < downloads.length) {
      event.preventDefault();
      const id = downloads[next]?.id ?? null;
      selected = id;
      (document.getElementById(`row-${id}`) as HTMLElement | null)?.focus();
    } else if (event.key === "Enter") {
      const id = downloads[index]?.id;
      if (id !== undefined) onopen(id);
    }
  }
</script>

<div class="scroll">
  <!-- A grid, so screen readers say which row is selected. -->
  <table role="grid" aria-label={locale.t("nav-all")}>
    <thead>
      <tr>
        <th scope="col">{locale.t("column-name")}</th>
        <th scope="col">{locale.t("column-size")}</th>
        <th scope="col">{locale.t("column-progress")}</th>
        <th scope="col">{locale.t("column-speed")}</th>
        <th scope="col">{locale.t("column-left")}</th>
        <th scope="col">{locale.t("column-status")}</th>
      </tr>
    </thead>
    <tbody>
      {#each downloads as d, i (d.id)}
        <tr
          id="row-{d.id}"
          tabindex={i === current ? 0 : -1}
          aria-selected={selected === d.id}
          onclick={() => (selected = d.id)}
          ondblclick={() => onopen(d.id)}
          onkeydown={(e) => onkeydown(e, i)}
        >
          <td><div class="name"><span class="type" aria-hidden="true">{extension(d.name)}</span><FileName name={d.name} /></div></td>
          <td class="num muted">{d.size === null ? "" : size(locale.lang, locale.digits, d.size)}</td>
          <td>
            <span class="progress">
              <Feathers
                parts={d.parts}
                status={d.status}
                label={locale.t("progress-label", {
                  percent: percent(locale.lang, locale.digits, fraction(d)),
                  count: d.segments,
                })}
              />
              <span class="pct num" aria-hidden="true">{percent(locale.lang, locale.digits, fraction(d))}</span>
            </span>
          </td>
          <td class="num">{speed(locale.lang, locale.digits, d.speed)}</td>
          <td class="num muted">{d.size === null ? "" : left(locale.lang, locale.digits, d.size - d.done, d.speed)}</td>
          <td><StatusChip status={d.status} browser={d.waits_for_browser} held={d.needs_ok} /></td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .scroll {
    overflow: auto;
    flex: 1;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    text-align: start;
    font-size: 12px;
    font-weight: 600;
    color: var(--faint);
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
    position: sticky;
    top: 0;
    background: var(--surface);
  }
  td {
    padding: 9px 12px;
    border-bottom: 1px solid color-mix(in oklab, var(--line) 60%, transparent);
    vertical-align: middle;
    white-space: nowrap;
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
  .name {
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 360px;
  }
  .type {
    width: 30px;
    height: 30px;
    border-radius: 7px;
    display: grid;
    place-items: center;
    background: var(--sunken);
    color: var(--muted);
    font-size: 10px;
    font-weight: 700;
    flex: none;
    border: 1px solid var(--line);
  }
  .muted {
    color: var(--muted);
  }
  .progress {
    display: inline-flex;
    align-items: center;
    gap: 10px;
  }
  .pct {
    font-size: 12px;
    color: var(--muted);
    min-width: 34px;
  }
</style>
