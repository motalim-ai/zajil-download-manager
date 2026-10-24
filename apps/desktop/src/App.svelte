<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Mark from "./lib/components/Mark.svelte";
  import { api } from "./lib/api";
  import { locale } from "./lib/i18n.svelte";
  import { size, speed } from "./lib/format";
  import type { Snapshot } from "./lib/bindings/Snapshot";

  let snapshot = $state<Snapshot>({ downloads: [], queues: [], categories: [], speed_limit: null, problem: null });
  let error = $state<string | null>(null);

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

<!-- The window's frame: the brand, the main area (the list comes in the
     next part), and the footer with what runs and waits. -->
<div class="window">
  <header class="brand"><Mark size={28} /><span>{locale.t("app-name")}</span></header>
  <main class="main">
    {#if error}<p class="problem" role="alert">{error}</p>{/if}
  </main>
  <footer class="status">
    <span><b class="num">{locale.t("footer-active", { count: active.length })}</b></span>
    <span>{locale.t("footer-total-speed", { speed: speed(locale.lang, locale.digits, totalSpeed) || size(locale.lang, locale.digits, 0) })}</span>
    <span>{snapshot.speed_limit ? locale.t("footer-limit", { speed: speed(locale.lang, locale.digits, snapshot.speed_limit) }) : locale.t("footer-limit-none")}</span>
    <span class="end">{locale.t("footer-waiting", { count: waiting })}</span>
  </footer>
</div>

<style>
  .window {
    display: grid;
    grid-template-rows: auto 1fr auto;
    height: 100%;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    font-weight: 700;
    font-size: 17px;
    border-bottom: 1px solid var(--line);
  }
  .main {
    min-height: 0;
  }
  .problem {
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
