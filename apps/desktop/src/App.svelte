<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Sidebar, { type Filter } from "./lib/components/Sidebar.svelte";
  import DownloadTable from "./lib/components/DownloadTable.svelte";
  import Icon from "./lib/components/Icon.svelte";
  import { api } from "./lib/api";
  import { locale } from "./lib/i18n.svelte";
  import { size, speed } from "./lib/format";
  import type { Snapshot } from "./lib/bindings/Snapshot";

  let snapshot = $state<Snapshot>({ downloads: [], queues: [], categories: [], speed_limit: null, problem: null });
  let filter = $state<Filter>({ kind: "all" });
  let selected = $state<number | null>(null);
  let error = $state<string | null>(null);

  const shown = $derived(
    snapshot.downloads.filter((d) => {
      switch (filter.kind) {
        case "all":
          return true;
        case "active":
          return d.status === "active";
        case "done":
          return d.status === "done";
        case "category":
          return d.category === filter.id;
        case "queue":
          return d.queue === filter.name;
      }
    }),
  );
  const active = $derived(snapshot.downloads.filter((d) => d.status === "active"));
  const waiting = $derived(snapshot.downloads.filter((d) => d.status === "waiting").length);
  const totalSpeed = $derived(active.reduce((sum, d) => sum + d.speed, 0));

  $effect(() => {
    // The window title follows the language.
    void getCurrentWindow().setTitle(locale.t("app-name")).catch(() => {});
  });

  // Snapshots can come back out of order: only the latest asked counts
  // (review of #45: an older one could win and stay).
  let asked = 0;
  async function refresh() {
    const mine = ++asked;
    try {
      const next = await api.snapshot();
      if (mine !== asked) return;
      snapshot = next;
      error = snapshot.problem;
    } catch (e) {
      if (mine === asked) error = String(e);
    }
  }

  async function act(kind: "pause" | "resume" | "remove") {
    if (selected === null) return;
    await api[kind]([selected]);
    await refresh();
  }

  onMount(() => {
    locale.set("ar");
    let unlisten: (() => void) | undefined;
    void (async () => {
      // Listening first, so a change during the first snapshot is not lost.
      unlisten = await api.onChange(() => void refresh());
      await refresh();
    })();
    return () => unlisten?.();
  });
</script>

<div class="window">
  <Sidebar {snapshot} bind:filter />
  <main class="main">
    <div class="tool">
      <button class="btn icon" aria-label={locale.t("action-resume")} disabled={selected === null} onclick={() => void act("resume")}><Icon name="play" /></button>
      <button class="btn icon" aria-label={locale.t("action-pause")} disabled={selected === null} onclick={() => void act("pause")}><Icon name="pause" /></button>
      <button class="btn icon" aria-label={locale.t("action-delete")} disabled={selected === null} onclick={() => void act("remove")}><Icon name="trash" /></button>
    </div>
    {#if error}<p class="problem" role="alert"><Icon name="alert" />{error}</p>{/if}
    <DownloadTable downloads={shown} bind:selected />
    <footer class="status">
      <span><b class="num">{locale.t("footer-active", { count: active.length })}</b></span>
      <span>{locale.t("footer-total-speed", { speed: speed(locale.lang, locale.digits, totalSpeed) || size(locale.lang, locale.digits, 0) })}</span>
      <span>{snapshot.speed_limit ? locale.t("footer-limit", { speed: speed(locale.lang, locale.digits, snapshot.speed_limit) }) : locale.t("footer-limit-none")}</span>
      <span class="end">{locale.t("footer-waiting", { count: waiting })}</span>
    </footer>
  </main>
</div>

<style>
  .window {
    display: grid;
    grid-template-columns: 236px 1fr;
    height: 100%;
  }
  .main {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tool {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 8px 16px 0;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--red-bg);
    color: var(--red);
  }
  .status {
    display: flex;
    gap: 18px;
    align-items: center;
    padding: 7px 16px;
    border-top: 1px solid var(--line);
    font-size: 12.5px;
    color: var(--muted);
  }
  .status b {
    color: var(--ink);
    font-weight: 600;
  }
  .end {
    margin-inline-start: auto;
  }
</style>
