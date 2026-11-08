<script lang="ts">
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import { ltr, toolLines } from "../about";
  import { api } from "../api";
  import { updateApi } from "../updateApi";
  import type { ToolView } from "../bindings/ToolView";
  import type { UpdateView } from "../bindings/UpdateView";

  let { onclose }: { onclose: () => void } = $props();
  let version = $state<string | null>(null);
  let versionError = $state<string | null>(null);
  let tools = $state<ToolView[]>([]);
  let toolsError = $state<string | null>(null);
  let found = $state<UpdateView | null>(null);
  let checking = $state(false);
  let checkMessage = $state<string | null>(null);
  let checkError = $state<string | null>(null);
  let installError = $state<string | null>(null);
  let releaseError = $state<string | null>(null);
  let copyMessage = $state<string | null>(null);
  let progress = $state<number | null | undefined>(undefined);
  let dialog: HTMLDivElement | undefined = $state();
  let manuallyChecked = false;

  const toolText = $derived(toolLines((key, args) => locale.t(key, args), tools).join("\n"));

  $effect(() => {
    void tick().then(() => dialog?.querySelector<HTMLButtonElement>("[data-close]")?.focus());
  });

  onMount(() => {
    const unlisten: (() => void)[] = [];
    void (async () => {
      unlisten.push(await updateApi.onFound((view) => (found = view)));
      unlisten.push(await updateApi.onProgress((event) => {
        progress = event.total && event.total > 0 ? Math.min(1, event.done / event.total) : null;
      }));
      void api.appVersion().then(
        (value) => (version = value),
        (error) => (versionError = String(error)),
      );
      void api.videoToolsStatus().then(
        (value) => (tools = value),
        (error) => (toolsError = String(error)),
      );
      void updateApi.notice().then(
        (view) => {
          if (!manuallyChecked) found = view;
        },
        (error) => (checkError = String(error)),
      );
    })().catch((error) => (checkError = String(error)));
    return () => unlisten.forEach((stop) => stop());
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onclose();
    }
  }

  async function checkForUpdates() {
    manuallyChecked = true;
    checking = true;
    checkMessage = null;
    checkError = null;
    installError = null;
    try {
      found = await updateApi.checkNow();
      checkMessage = found ? null : locale.t("about-up-to-date");
    } catch (error) {
      checkError = String(error);
    } finally {
      checking = false;
    }
  }

  async function installUpdate() {
    installError = null;
    progress = null;
    try {
      await updateApi.install();
    } catch (error) {
      progress = undefined;
      installError = String(error);
    }
  }

  async function openReleasePage() {
    releaseError = null;
    try {
      await updateApi.openReleasePage();
    } catch (error) {
      releaseError = String(error);
    }
  }

  async function copyProblemInfo() {
    copyMessage = null;
    try {
      await api.copyProblemInfo();
      copyMessage = locale.t("about-copied-problem");
    } catch {
      copyMessage = locale.t("about-copy-failed");
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="scrim">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="about-title" tabindex="-1" bind:this={dialog}>
    <header>
      <h2 id="about-title">{locale.t("cmd-about")}</h2>
      <button class="btn icon quiet" data-close aria-label={locale.t("about-close")} title={locale.t("about-close")} onclick={onclose}>
        <Icon name="x" />
      </button>
    </header>
    <div class="content">
      {#if version !== null}
        <p class="version">{locale.t("about-text", { version: ltr(version) })}</p>
      {:else if versionError}
        <p class="problem" role="alert">{locale.t("about-version-error", { error: versionError })}</p>
      {/if}
      {#if toolsError}
        <p class="problem" role="alert">{locale.t("about-tools-error", { error: toolsError })}</p>
      {:else if toolText}
        <pre dir="auto">{toolText}</pre>
      {/if}

      <section class="updates" aria-label={locale.t("about-updates")}>
        <h3>{locale.t("about-updates")}</h3>
        {#if found}
          <p>{locale.t("about-update-found", { version: ltr(found.version) })}</p>
          {#if progress !== undefined}
            <p role="status">{locale.t("about-update-installing")} {#if progress !== null}<bdi dir="ltr">{Math.round(progress * 100)}%</bdi>{/if}</p>
          {:else if found.can_install}
            <button class="btn primary" onclick={() => void installUpdate()}><Icon name="download" />{locale.t("update-install")}</button>
          {:else}
            <button class="btn" onclick={() => void openReleasePage()}><Icon name="external-link" />{locale.t("update-release-page")}</button>
          {/if}
        {:else if checkMessage}
          <p role="status">{checkMessage}</p>
        {/if}
        {#if checkError}<p class="problem" role="alert">{locale.t("about-check-failed", { error: checkError })}</p>{/if}
        {#if installError}<p class="problem" role="alert">{locale.t("about-install-failed", { error: installError })}</p>{/if}
        {#if releaseError}<p class="problem" role="alert">{locale.t("about-release-failed", { error: releaseError })}</p>{/if}
        <button class="btn" disabled={checking} onclick={() => void checkForUpdates()}>
          <Icon name="rotate-ccw" />{locale.t(checking ? "about-checking-updates" : "about-check-updates")}
        </button>
      </section>
    </div>
    <footer>
      {#if copyMessage}<span role="status">{copyMessage}</span>{/if}
      <button class="btn" onclick={() => void copyProblemInfo()}>{locale.t("about-copy-problem")}</button>
      <button class="btn" data-close onclick={onclose}>{locale.t("about-close")}</button>
    </footer>
  </div>
</div>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 30;
    display: grid;
    place-items: center;
    padding: 16px;
    overflow: auto;
    background: var(--scrim);
  }
  .dialog {
    display: flex;
    flex-direction: column;
    width: min(560px, 100%);
    max-height: 100%;
    min-height: 0;
    padding: 18px 20px 16px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  header h2 {
    flex: 1;
    margin: 0;
    font-size: 17px;
  }
  .content {
    min-height: 0;
    overflow: auto;
  }
  .version {
    margin: 8px 0;
    font-weight: 600;
  }
  pre {
    margin: 8px 0;
    font: inherit;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .updates {
    padding-top: 8px;
    border-top: 1px solid var(--line);
  }
  .updates h3 {
    margin: 0;
    font-size: 14px;
  }
  .updates p {
    margin: 6px 0;
  }
  .problem {
    color: var(--red);
    overflow-wrap: anywhere;
  }
  footer {
    justify-content: flex-end;
    flex-wrap: wrap;
    padding-top: 14px;
  }
  footer span {
    flex: 1 1 100%;
    color: var(--muted);
    font-size: 12px;
  }
</style>
