<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import { ask } from "@tauri-apps/plugin-dialog";
  import Icon from "../lib/components/Icon.svelte";
  import ColorIcon from "../lib/components/ColorIcon.svelte";
  import { isolatedName } from "../lib/fname";
  import { api } from "../lib/api";
  import { locale } from "../lib/i18n.svelte";
  import { counted, left, num, percent, size, speed } from "../lib/format";
  import { startWindow } from "../lib/windowStart";
  import type { DetailsView } from "../lib/bindings/DetailsView";

  // A download's progress window (M5.5, the design of #83, window 4), one
  // per download as in IDM: its state, its own speed limit, and what to do
  // once it finishes; the details show its parts and their connections.
  // The window's label says which download (`progress-<id>`).
  let { id }: { id: number } = $props();

  let details = $state<DetailsView | null>(null);
  let problem = $state<string | null>(null);
  let tab = $state<"status" | "limit" | "finish">("status");
  // Shown or not, as the user left it last (a convenience: per viewer).
  let more = $state(readMore());
  // When finished (this window acts, while it is open).
  // From the settings, for every download (owner's request, 2026-10-30);
  // changed here, for this one only.
  let openFile = $state(false);
  let openFolder = $state(false);
  let closeWhenDone = $state(true);
  // The speed limit tab: KB/s, or none.
  let limited = $state(false);
  let limitKb = $state(512);
  let limitSaid = $state<string | null>(null);

  function readMore(): boolean {
    try {
      return localStorage.getItem("zajil.progress.more") !== "0";
    } catch {
      return true;
    }
  }

  const d = $derived(details?.download);
  const total = $derived(d?.size ?? 0);
  const fraction = $derived(d ? Math.min(1, total ? d.done / total : d.status === "done" ? 1 : 0) : 0);
  const running = $derived(d?.status === "active");
  // A part's connection is at work while the download runs and the part
  // is not whole.
  const whole = (s: DetailsView["segments"][number]) =>
    d?.status === "done" || (s.end !== null && s.start + s.done > s.end);
  const working = $derived(running && details ? details.segments.filter((s) => !whole(s)).length : 0);

  // The speed of the last reads, drawn as a line under the bar (the
  // owner's request: a window that shows it at work).
  const SPEEDS = 60;
  let speeds = $state<number[]>([]);
  const speedLine = $derived.by(() => {
    if (speeds.length < 2) return "";
    const top = Math.max(...speeds) || 1;
    const step = 100 / (SPEEDS - 1);
    const at = (v: number, i: number) => `${((SPEEDS - speeds.length + i) * step).toFixed(2)},${(30 - (v / top) * 27).toFixed(2)}`;
    return speeds.map(at).join(" ");
  });

  // One read at a time, and one more after it if changes came meanwhile
  // (review of #111: every change of any download read three files).
  let loading = false;
  let again = false;
  let gone = $state(false);
  async function load() {
    if (gone) return;
    if (loading) {
      again = true;
      return;
    }
    loading = true;
    try {
      const next = await api.details(id);
      const was = details?.download.status;
      if (next.download.status === "active") speeds = [...speeds.slice(1 - SPEEDS), next.download.speed];
      details = next;
      problem = null;
      if (was && was !== "done" && next.download.status === "done") void finished();
    } catch (e) {
      // Taken off the list: nothing more to show (review of #111).
      if (/^no download \d+$/.test(String(e))) gone = true;
      else problem = String(e);
    } finally {
      loading = false;
      if (again && !gone) {
        again = false;
        void load();
      }
    }
  }

  // The title: the percentage, then the name (as IDM); set only when it
  // changes.
  let title = "";
  $effect(() => {
    if (!d) return;
    const shown = d.status === "done" ? locale.t("status-done") : percent(locale.lang, locale.digits, fraction);
    const next = `${shown} ${isolatedName(d.name)}`;
    if (next === title) return;
    title = next;
    void getCurrentWindow()
      .setTitle(next)
      .catch(() => {});
  });

  // A video has no parts to show (#149). A value of its own: the effect
  // below runs only when it or `more` changes, not at every update of the
  // download (the owner's notes on 1.0: it took the user's height back to
  // 640 four times a second, so the window could not be made taller).
  const video = $derived(d?.video === true);

  // With the details, the window is taller; its width stays the user's.
  $effect(() => {
    const height = more && !video ? 640 : 430;
    // The page's own width is the window's, in logical pixels: no
    // permission to read the window (review of #111).
    void getCurrentWindow()
      .setSize(new LogicalSize(window.innerWidth || 600, height))
      .catch(() => {});
    try {
      localStorage.setItem("zajil.progress.more", more ? "1" : "0");
    } catch {
      // Not kept.
    }
  });

  // The kept limit fills the tab when it changes, not at every update
  // (review of #111: what the user typed was taken back four times a
  // second).
  let shownLimit: number | null | undefined = undefined;
  $effect(() => {
    if (!details || details.limit === shownLimit) return;
    shownLimit = details.limit;
    limited = details.limit !== null;
    if (details.limit !== null) limitKb = Math.max(1, Math.round(details.limit / 1024));
  });

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      await startWindow();
      const finish = await api.finishSetting().catch(() => null);
      if (finish) {
        openFile = finish.open_file;
        openFolder = finish.open_folder;
        closeWhenDone = finish.close_window;
      }
      unlisten = await api.onChange(() => void load());
      await load();
      await tick();
      document.getElementById("progress-pause")?.focus();
    })();
    return () => unlisten?.();
  });

  async function finished() {
    // A file that can run code opens once the user agrees (#84).
    if (openFile && !(await api.openDownload(id, false).catch(() => true))) {
      const yes = await ask(locale.t("open-code-question", { name: d?.name ?? "" }), {
        title: locale.t("app-name"),
        kind: "warning",
        okLabel: locale.t("open-code-ok"),
        cancelLabel: locale.t("confirm-cancel"),
      }).catch(() => false);
      if (yes) await api.openDownload(id, false, true).catch(() => {});
    }
    if (openFolder) await api.openDownload(id, true).catch(() => {});
    if (closeWhenDone) await api.closeWindow();
  }

  async function act(kind: "pause" | "resume") {
    problem = null;
    try {
      await api[kind]([id]);
    } catch (e) {
      problem = String(e);
    }
    await load();
  }
  // «Cancel»: the download stops (it stays in the list), and the window
  // closes, as in IDM.
  async function cancel() {
    if (d && (d.status === "active" || d.status === "waiting")) await api.pause([id]).catch(() => {});
    await api.closeWindow();
  }
  // KB/s: a whole number from 1 to 10 GB/s; anything else is said, not
  // kept (review of #111: an empty box kept 1 KB/s). Applied as soon as it
  // changes, with no «Save» (the owner's notes on 1.0); while the user
  // types, a number that is not one yet waits quietly.
  const MAX_KB = 10 * 1024 * 1024;
  let limitTimer: ReturnType<typeof setTimeout> | undefined;
  function limitChanged(typing: boolean) {
    clearTimeout(limitTimer);
    limitTimer = setTimeout(() => void applyLimit(!typing), typing ? 400 : 0);
  }
  async function applyLimit(sayBad: boolean) {
    const kb = Number(limitKb);
    if (limited && !(Number.isFinite(kb) && kb >= 1 && kb <= MAX_KB)) {
      if (sayBad) limitSaid = locale.t("progress-limit-bad", { min: num(locale.lang, locale.digits, 1, 0), max: num(locale.lang, locale.digits, MAX_KB, 0) });
      return;
    }
    const bytes = limited ? Math.round(kb) * 1024 : null;
    if (bytes === details?.limit) return;
    limitSaid = null;
    try {
      await api.setLimit(id, bytes);
      limitSaid = locale.t(running ? "progress-limit-now" : "progress-limit-saved");
      await load();
    } catch (e) {
      limitSaid = String(e);
    }
  }

  function statusText(): string {
    if (!d) return "…";
    if (d.needs_ok) return locale.t("status-needs-ok");
    if (d.waits_for_browser) return locale.t("status-open-browser");
    if (d.status === "active") return locale.t("progress-receiving");
    if (d.status === "failed" && d.error) return locale.t("details-failed", { error: d.error });
    return locale.t(`status-${d.status}`);
  }

  const TABS = [
    ["status", "progress-tab-status"],
    ["limit", "progress-tab-limit"],
    ["finish", "progress-tab-finish"],
  ] as const;
  // A video has no speed limit of its own (yt-dlp runs it, #149).
  const tabs = $derived(TABS.filter(([key]) => key !== "limit" || !d?.video));
  function onTabKey(event: KeyboardEvent) {
    const order = tabs.map(([t]) => t);
    const at = order.indexOf(tab);
    const forward = locale.dir === "rtl" ? "ArrowLeft" : "ArrowRight";
    const back = locale.dir === "rtl" ? "ArrowRight" : "ArrowLeft";
    if (event.key !== forward && event.key !== back) return;
    event.preventDefault();
    tab = order[(at + (event.key === forward ? 1 : -1) + order.length) % order.length]!;
    void tick().then(() => document.getElementById(`tab-${tab}`)?.focus());
  }
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      void api.closeWindow();
    }
  }
</script>

<svelte:window {onkeydown} />

<main class="progress" aria-labelledby="progress-name">
  {#if gone}
    <p class="gone" role="status">{locale.t("progress-gone")}</p>
  {/if}
  <h1 id="progress-name" class="visually-hidden">{d ? d.name : locale.t("progress-title")}</h1>
  <div class="tabs" role="tablist" aria-label={locale.t("progress-title")}>
    {#each tabs as [key, text] (key)}
      <button
        id="tab-{key}"
        role="tab"
        class="tab"
        aria-selected={tab === key}
        aria-controls={tab === key ? `panel-${key}` : undefined}
        tabindex={tab === key ? 0 : -1}
        onclick={() => (tab = key)}
        onkeydown={onTabKey}>{locale.t(text)}</button
      >
    {/each}
  </div>

  <div class="top">
    {#if tab === "status"}
      <div id="panel-status" role="tabpanel" aria-labelledby="tab-status" class="panel">
        <dl class="facts">
          <dt>{locale.t("add-url")}</dt>
          <dd class="url"><bdi dir="ltr">{details?.url ?? ""}</bdi></dd>
          <dt>{locale.t("progress-status")}</dt>
          <dd class:bad={d?.status === "failed"}>{statusText()}</dd>
          <dt>{locale.t("progress-size")}</dt>
          <dd class="num">{d?.size === null || !d ? locale.t("details-unknown") : size(locale.lang, locale.digits, d.size)}</dd>
          <dt>{locale.t("progress-done")}</dt>
          <dd class="num">{d ? `${size(locale.lang, locale.digits, d.done)} (${percent(locale.lang, locale.digits, fraction)})` : ""}</dd>
          <dt>{locale.t("details-speed")}</dt>
          <dd class="num">{running && d ? speed(locale.lang, locale.digits, d.speed) : "—"}</dd>
          <dt>{locale.t("col-left")}</dt>
          <dd class="num">{running && d && d.size !== null ? left(locale.lang, locale.digits, Math.max(0, d.size - d.done), d.speed) : "—"}</dd>
          <dt>{locale.t("progress-resume")}</dt>
          <dd>
            {#if details?.resumable === true}<strong class="yes">{locale.t("progress-resume-yes")}</strong>
            {:else if details?.resumable === false}<strong class="no">{locale.t("progress-resume-no")}</strong>
            {:else}{locale.t("details-unknown")}{/if}
          </dd>
        </dl>
        <div class="bar" class:going={running} role="progressbar" aria-label={locale.t("progress-done")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(fraction * 100)}>
          <b style:inline-size="{fraction * 100}%"></b>
        </div>
        {#if speedLine}
          <svg class="speeds" viewBox="0 0 100 30" preserveAspectRatio="none" aria-hidden="true">
            <polygon points="{speedLine.split(" ")[0]!.split(",")[0]},30 {speedLine} 100,30" />
            <polyline points={speedLine} />
          </svg>
        {/if}
      </div>
    {:else if tab === "limit"}
      <div id="panel-limit" role="tabpanel" aria-labelledby="tab-limit" class="panel">
        <label class="check"><input type="checkbox" bind:checked={limited} onchange={() => limitChanged(false)} />{locale.t("progress-limit-on")}</label>
        <label class="row" class:off={!limited}>
          <input type="number" min="1" step="1" bind:value={limitKb} disabled={!limited} dir="ltr" oninput={() => limitChanged(true)} onchange={() => limitChanged(false)} />
          <span>{locale.t("progress-limit-unit")}</span>
        </label>
        <p class="hint">{locale.t("progress-limit-hint")}</p>
        {#if limitSaid}<p class="said" role="status">{limitSaid}</p>{/if}
      </div>
    {:else}
      <div id="panel-finish" role="tabpanel" aria-labelledby="tab-finish" class="panel">
        <p class="hint">{locale.t("progress-finish-hint")}</p>
        <label class="check"><input type="checkbox" bind:checked={openFile} />{locale.t("progress-finish-open")}</label>
        <label class="check"><input type="checkbox" bind:checked={openFolder} />{locale.t("progress-finish-folder")}</label>
        <label class="check"><input type="checkbox" bind:checked={closeWhenDone} />{locale.t("progress-finish-close")}</label>
      </div>
    {/if}

    {#if problem}<p class="problem" role="alert"><ColorIcon name="s-failed" size={17} />{problem}</p>{/if}

    <div class="actions">
      {#if !d?.video}
        <button class="btn" aria-expanded={more} aria-controls="progress-more" onclick={() => (more = !more)}>
          <Icon name={more ? "chevron-up" : "chevron-down"} size={15} />{locale.t(more ? "progress-less" : "progress-more")}
        </button>
      {/if}
      <span class="gap"></span>
      {#if gone}
        <!-- Nothing to pause or resume. -->
      {:else if d && (d.status === "active" || d.status === "waiting")}
        <button class="btn" id="progress-pause" onclick={() => void act("pause")}><Icon name="pause" />{locale.t("action-pause")}</button>
      {:else if d && d.status !== "done"}
        <button class="btn" id="progress-pause" onclick={() => void act("resume")}><Icon name="play" />{locale.t("action-resume")}</button>
      {/if}
      <button class="btn" onclick={() => void cancel()}>{locale.t(d?.status === "done" ? "progress-close" : "add-cancel")}</button>
    </div>
  </div>

  {#if more && !d?.video}
    <section id="progress-more" class="more" aria-labelledby="parts-title">
      <div class="head">
        <h2 id="parts-title">{locale.t("details-parts")}</h2>
        {#if details && details.segments.length > 0}
          <span class="muted">{locale.t("progress-connections", { connections: counted(locale.lang, locale.digits, "connections-count", details.segments.length), working: locale.t("progress-working", { number: num(locale.lang, locale.digits, working, 0) }) })}</span>
        {/if}
      </div>
      {#if details && total > 0 && details.segments.length > 0}
        <div class="parts" role="img" aria-label={locale.t("details-parts")}>
          {#each details.segments as s (s.start)}
            {@const from = (s.start / total) * 100}
            {@const width = (Math.min(s.done, (s.end ?? total - 1) - s.start + 1) / total) * 100}
            <span class="got" style:inset-inline-start="{from}%" style:inline-size="{width}%" class:writing={running && !whole(s)}></span>
            <!-- Where each connection started. -->
            <span class="start" style:inset-inline-start="{from}%"></span>
          {/each}
        </div>
        <div class="legend">
          <span><i class="got"></i>{locale.t("details-map-done")}</span>
          <span><i class="writing"></i>{locale.t("details-map-writing")}</span>
          <span><i></i>{locale.t("progress-left-part")}</span>
        </div>
        <table class="conns">
          <thead>
            <tr><th scope="col">{locale.t("progress-connection")}</th><th scope="col">{locale.t("progress-got")}</th><th scope="col">{locale.t("progress-status")}</th></tr>
          </thead>
          <tbody>
            {#each details.segments as s, i (s.start)}
              {@const length = s.end === null ? 0 : s.end - s.start + 1}
              <tr>
                <td class="num">{num(locale.lang, locale.digits, i + 1, 0)}</td>
                <td class="num">{size(locale.lang, locale.digits, s.done)}</td>
                <td>
                  <span class="state" class:yes={whole(s)} class:live={running && !whole(s)}>
                    <ColorIcon name={whole(s) ? "s-done" : running ? "s-active" : "s-paused"} size={13} />
                    {#if length > 0}<span class="mini" aria-hidden="true"><b style:inline-size="{Math.min(100, (s.done / length) * 100)}%"></b></span>{/if}
                    {whole(s) ? locale.t("details-part-done") : running ? locale.t("progress-receiving") : locale.t(`status-${d?.status ?? "paused"}`)}
                  </span>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="muted">{locale.t(d?.status === "done" ? "progress-parts-done" : "progress-no-parts")}</p>
      {/if}
    </section>
  {/if}
</main>

<style>
  .gone {
    margin: 0;
    padding: 8px 20px;
    background: var(--sunken);
    color: var(--muted);
    font-size: 13px;
  }
  .progress {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--surface);
    box-sizing: border-box;
    overflow: auto;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 8px 16px 0;
    border-bottom: 1px solid var(--line);
    font-size: 13px;
  }
  .tab {
    padding: 6px 12px;
    border-radius: 4px 4px 0 0;
    color: var(--muted);
    border: 1px solid transparent;
    border-bottom: 0;
    margin-bottom: -1px;
    background: none;
    font: inherit;
    cursor: pointer;
  }
  .tab[aria-selected="true"] {
    color: var(--ink);
    font-weight: 600;
    background: var(--surface);
    border-color: var(--line);
  }
  .top {
    padding: 16px 20px;
  }
  .panel {
    min-height: 236px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 24px;
    margin: 0;
    font-size: 13.5px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    min-width: 0;
  }
  .url bdi {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .yes {
    color: var(--green);
  }
  .no,
  .bad {
    color: var(--amber);
  }
  /* A desktop progress bar: green, with moving stripes while it runs. */
  .bar {
    height: 16px;
    border-radius: 3px;
    background: var(--sunken);
    box-shadow: inset 0 0 0 1px var(--line);
    overflow: hidden;
    margin-top: 6px;
  }
  .bar b {
    display: block;
    position: relative;
    overflow: hidden;
    height: 100%;
    border-radius: 3px;
    background: linear-gradient(#4fd977, #1f9e44);
  }
  /* The stripes on a layer of their own, moved by `transform`: the
     compositor moves them, with no repaint of the bar each frame (moving
     `background-position` repainted it sixty times a second while a
     download ran). One stripe wider than the bar, so the loop is seamless. */
  .bar.going b::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: -22.6px;
    right: 0;
    background: repeating-linear-gradient(-45deg, rgb(255 255 255 / 0.18) 0 8px, transparent 8px 16px);
    background-size: 22.6px 100%;
    animation: stripes 1s linear infinite;
    will-change: transform;
  }
  @keyframes stripes {
    to {
      transform: translateX(22.6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar.going b::after {
      animation: none;
    }
  }
  .speeds {
    display: block;
    width: 100%;
    height: 30px;
    margin-top: 4px;
  }
  .speeds polygon {
    fill: color-mix(in oklab, #1f9e44 18%, transparent);
  }
  .speeds polyline {
    fill: none;
    stroke: #1f9e44;
    stroke-width: 1.4;
    vector-effect: non-scaling-stroke;
  }
  .check,
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 13.5px;
  }
  .row.off {
    color: var(--faint);
  }
  .row input {
    width: 110px;
    font: inherit;
    padding: 5px 8px;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--sunken);
    color: var(--ink);
  }
  .hint,
  .said,
  .muted {
    margin: 0;
    font-size: 12.5px;
    color: var(--muted);
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--red);
    font-size: 13px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    padding-top: 12px;
  }
  .gap {
    flex: 1;
  }
  .more {
    border-top: 1px solid var(--line);
    background: var(--sunken);
    padding: 14px 20px 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  h2 {
    margin: 0;
    font-size: 13.5px;
  }
  /* Each connection's start (a red mark) and how far it came (green),
     on one line, as desktop download managers show it. */
  .parts {
    position: relative;
    height: 18px;
    border-radius: 2px;
    background: var(--surface);
    box-shadow: inset 0 0 0 1px var(--line);
    overflow: hidden;
  }
  .parts .got {
    position: absolute;
    top: 1px;
    bottom: 1px;
    background: linear-gradient(#4fd977, #1f9e44);
  }
  .parts .got.writing {
    box-shadow: inset -3px 0 0 #a8f0bd;
  }
  :global([dir="rtl"]) .parts .got.writing {
    box-shadow: inset 3px 0 0 #a8f0bd;
  }
  .parts .start {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: #d42f2c;
  }
  .legend {
    display: flex;
    gap: 16px;
    font-size: 12px;
    color: var(--muted);
  }
  .legend span {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .legend i {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    background: var(--line);
  }
  .legend i.got {
    background: #1f9e44;
  }
  .legend i.writing {
    background: #a8f0bd;
  }
  .conns {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
    background: var(--surface);
    border: 1px solid var(--line);
  }
  .conns th {
    text-align: start;
    font-weight: 600;
    color: var(--muted);
    padding: 3px 8px;
    border-bottom: 1px solid var(--line);
    background: linear-gradient(var(--surface), var(--sunken));
  }
  .conns td {
    padding: 2px 8px;
    border-bottom: 1px solid color-mix(in oklab, var(--line) 70%, transparent);
  }
  .conns th + th,
  .conns td + td {
    border-inline-start: 1px solid color-mix(in oklab, var(--line) 60%, transparent);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .state.live {
    color: var(--accent);
  }
  .state .mini {
    width: 60px;
    height: 6px;
    border-radius: 2px;
    background: var(--sunken);
    box-shadow: inset 0 0 0 1px var(--line);
    overflow: hidden;
  }
  .state .mini b {
    display: block;
    height: 100%;
    background: linear-gradient(#4fd977, #1f9e44);
  }
</style>
