<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import Settings from "../lib/components/Settings.svelte";
  import { api } from "../lib/api";
  import { locale, type Digits, type Lang } from "../lib/i18n.svelte";
  import { setTheme } from "../lib/theme";
  import { startWindow } from "../lib/windowStart";
  import type { SettingsView } from "../lib/bindings/SettingsView";
  import type { SettingsChange } from "../lib/bindings/SettingsChange";

  // The settings window (the owner's request: a window of its own with its
  // sections at its side, as in other download managers). What it saves
  // reaches the main window too (`settings-changed`).
  let settings = $state<SettingsView | null>(null);
  let problem = $state<string | null>(null);
  let message = $state<{ text: string; problem: boolean } | null>(null);
  let keepFolders = $state(false);

  function apply(next: SettingsView) {
    settings = next;
    locale.set(next.language as Lang);
    locale.digits = next.digits as Digits;
    setTheme(next.theme);
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      await startWindow();
      unlisten = await api.onSettingsChanged(apply);
      try {
        apply(await api.settings());
      } catch (e) {
        problem = String(e);
      }
    })();
    return () => unlisten?.();
  });

  async function change(next: SettingsChange) {
    try {
      apply(await api.saveSettings(next));
      message = null;
    } catch (e) {
      message = { text: String(e), problem: true };
    }
  }

  async function chooseFolder() {
    const chosen = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof chosen === "string" && settings) await change({ ...settings, download_dir: chosen });
  }

  async function exportQueues() {
    const path = await saveDialog({ defaultPath: "zajil-queues.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path) return;
    try {
      await api.exportQueues(path);
      message = { text: locale.t("export-done"), problem: false };
    } catch (e) {
      message = { text: String(e), problem: true };
    }
  }

  async function importQueues() {
    const path = await openDialog({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    try {
      const done = await api.importQueues(path, keepFolders);
      const kept = done.kept_settings.length ? " " + locale.t("import-kept", { names: done.kept_settings.join("، ") }) : "";
      message = { text: locale.t("import-done", { count: done.items_added }) + kept, problem: false };
    } catch (e) {
      message = { text: String(e), problem: true };
    }
  }
</script>

<div class="window">
  {#if settings}
    <Settings
      {settings}
      bind:keepFolders
      onchange={(c) => void change(c)}
      onchoosefolder={() => void chooseFolder()}
      onexport={() => void exportQueues()}
      onimport={() => void importQueues()}
      oncategories={() => void api.showMainView("categories")}
      {message}
    />
  {:else if problem}
    <p class="problem" role="alert">{problem}</p>
  {/if}
</div>

<style>
  .window {
    height: 100vh;
    background: var(--surface);
  }
  .problem {
    margin: 20px;
    color: var(--red);
  }
</style>
