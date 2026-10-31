<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Icon from "./Icon.svelte";
  import Mark from "./Mark.svelte";
  import { locale } from "../i18n.svelte";
  import { onMac } from "../keys";

  // The main window's title bar (the design of #83): the logo and «زاجِل
  // لإدارة التحميل» on the right in both languages (the owner's choice,
  // 2026-10-31: the logo keeps its place when the language changes). The
  // window is dragged by it and a double-click maximizes it
  // (`data-tauri-drag-region`).
  // - macOS: the system's bar is hidden under the page (`Overlay`); its
  //   coloured buttons are on the left, or on the right on a Mac in
  //   Arabic, so the bar leaves room for them on both sides.
  // - Windows and Linux: the window has no system frame
  //   (`tauri.windows.conf.json`, `tauri.linux.conf.json`): the minimize,
  //   maximize and close buttons are here, at the end of the reading
  //   direction. Close goes the way the system's does (into the tray, #109).
  const mac = onMac();
  // The Mac's coloured buttons follow the system's first language, not
  // Zajil's: on the right when it reads right to left.
  const buttonsRight = mac && /^(ar|he|fa|ur|ps|sd|ug|yi|dv|ckb)(-|$)/i.test(navigator.language);
  const win = getCurrentWindow();
  let maximized = $state(false);

  onMount(() => {
    if (mac) return;
    let unlisten: (() => void) | undefined;
    const check = () => void win.isMaximized().then((m) => (maximized = m), () => {});
    check();
    void win.onResized(check).then((u) => (unlisten = u), () => {});
    return () => unlisten?.();
  });
</script>

<!-- "deep": the logo and the name drag too; the buttons do not. -->
<!-- Laid out left to right whatever the language, so the brand stays on
     the right; the brand reads in the language's own direction. -->
<div class="titlebar" class:mac class:buttons-right={buttonsRight} class:rtl={locale.dir === "rtl"} dir="ltr" data-tauri-drag-region="deep">
  <span class="brand" dir={locale.dir}>
    <Mark size={18} />
    <span class="name">{locale.t("app-title")}</span>
  </span>
  {#if !mac}
    <span class="controls">
      <button class="ctl" aria-label={locale.t("win-minimize")} title={locale.t("win-minimize")} onclick={() => void win.minimize()}>
        <Icon name="minus" size={15} />
      </button>
      <button
        class="ctl"
        aria-label={locale.t(maximized ? "win-restore" : "win-maximize")}
        title={locale.t(maximized ? "win-restore" : "win-maximize")}
        onclick={() => void win.toggleMaximize()}
      >
        <Icon name={maximized ? "copy" : "square"} size={13} />
      </button>
      <button class="ctl close" aria-label={locale.t("win-close")} title={locale.t("win-close")} onclick={() => void win.close()}>
        <Icon name="x" size={16} />
      </button>
    </span>
  {/if}
</div>

<style>
  .titlebar {
    display: flex;
    align-items: center;
    height: 34px;
    padding-right: 12px;
    background: var(--frame, var(--sunken));
    border-bottom: 1px solid var(--line);
    user-select: none;
    -webkit-user-select: none;
    flex: none;
  }
  /* macOS: room for the coloured buttons on their side: the left, or the
     right on a Mac in Arabic (a user's report, 2026-10-30: they covered
     the logo). */
  .titlebar.mac {
    height: 30px;
    padding-left: 80px;
  }
  .titlebar.mac.buttons-right {
    padding-left: 12px;
    padding-right: 80px;
  }
  .brand {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 600;
    color: var(--ink);
  }
  /* Windows and Linux: the window's buttons stay at the end of the
     reading direction, on the right in English (after the brand) and on
     the left in Arabic. */
  .controls {
    display: flex;
    height: 100%;
    margin-left: 8px;
    margin-right: -12px;
  }
  .rtl .controls {
    order: -1;
    margin-left: 0;
    margin-right: 0;
  }
  .ctl {
    width: 46px;
    height: 100%;
    border: 0;
    background: none;
    color: var(--muted);
    display: grid;
    place-items: center;
    cursor: default;
  }
  .ctl:hover {
    background: var(--sel);
    color: var(--ink);
  }
  .ctl.close:hover {
    background: #c42b1c;
    color: #fff;
  }
  .ctl:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
</style>
