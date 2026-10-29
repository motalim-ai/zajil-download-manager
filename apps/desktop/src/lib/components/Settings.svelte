<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import { num } from "../format";
  import type { SettingsView } from "../bindings/SettingsView";
  import type { SettingsChange } from "../bindings/SettingsChange";
  import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
  import { onMount, untrack } from "svelte";
  import { api } from "../api";
  import { play } from "../sounds";
  import type { Sounds } from "../bindings/Sounds";
  import type { AddPrefs } from "../bindings/AddPrefs";
  import type { IntegrationView } from "../bindings/IntegrationView";
  import CompleteSetting from "./CompleteSetting.svelte";
  import UpdateSetting from "./UpdateSetting.svelte";
  import SiteLimitsSetting from "./SiteLimitsSetting.svelte";
  import type { ToolView } from "../bindings/ToolView";
  import { licenseOf, toolName, versionOf } from "../about";
  import { ask } from "@tauri-apps/plugin-dialog";

  let {
    settings,
    onchange,
    onchoosefolder,
    onexport,
    onimport,
    oncategories,
    message,
    keepFolders = $bindable(),
  }: {
    settings: SettingsView;
    /** Imports use the folders in the file (otherwise the download folder). */
    keepFolders: boolean;
    onchange: (change: SettingsChange) => void;
    onchoosefolder: () => void;
    onexport: () => void;
    onimport: () => void;
    /** Opens the categories editor (M5.5: its way in, review of #96). */
    oncategories: () => void;
    /** What the last action said (an import's summary, or an error). */
    message: { text: string; problem: boolean } | null;
  } = $props();

  const KIB = 1024;
  const MIB = 1024 * 1024;

  // The limit as the user types it: a number and a unit.
  let limitUnit = $state<"kib" | "mib">("mib");
  let limitValue = $state("");
  $effect(() => {
    const limit = settings.speed_limit;
    if (limit === null) return;
    // What the user typed already says it (applied while typing): kept as
    // typed, unit and all.
    const typed = untrack(() => Math.round(Number(limitValue) * (limitUnit === "mib" ? MIB : KIB)));
    if (typed === limit) return;
    limitUnit = limit % MIB === 0 ? "mib" : "kib";
    limitValue = String(limit / (limitUnit === "mib" ? MIB : KIB));
  });

  function change(patch: Partial<SettingsChange>) {
    onchange({
      language: settings.language,
      theme: settings.theme,
      digits: settings.digits,
      download_dir: settings.download_dir,
      connections: settings.connections,
      notifications: settings.notifications,
      clipboard: settings.clipboard,
      update_check: settings.update_check,
      site_connections: settings.site_connections,
      speed_limit: settings.speed_limit,
      ...patch,
    });
  }

  function setConnections(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const n = Math.round(Number(input.value));
    if (Number.isFinite(n) && n >= 1 && n <= settings.max_connections) change({ connections: n });
    else input.value = String(settings.connections);
  }

  // The browser integration (M5): how the last registration went, the one
  // at the start until the button is pressed. The button stays focusable
  // while it works (aria-disabled), and one live region says the result.
  const BROWSERS: Record<string, string> = { chrome: "Chrome", chromium: "Chromium", edge: "Microsoft Edge", firefox: "Firefox" };
  const HOST_PROBLEMS = { missing: "settings-browser-missing", temporary: "settings-browser-temporary", unsafe: "settings-browser-unsafe" } as const;
  let integration = $state<IntegrationView | null>(null);
  let integrationProblem = $state<string | null>(null);
  let fixing = $state(false);
  // Closing the window: hides it into the tray, unless the user chose to
  // quit (or the desktop has no tray, where it always quits).
  let closeQuits = $state(false);
  let hasTray = $state(true);
  onMount(() => {
    api.closeQuits().then(
      ([quits, tray]) => ((closeQuits = quits), (hasTray = tray)),
      () => {},
    );
  });
  // The drop box (M5.5): off at first.
  let dropBox = $state(false);
  onMount(() => {
    api.dropBoxShown().then(
      (shown) => (dropBox = shown),
      () => {},
    );
  });
  async function setDropBox(shown: boolean) {
    const before = dropBox;
    dropBox = shown;
    await api.setDropBox(shown).catch(() => (dropBox = before));
  }
  // How a new download shows (M5.5).
  const PROGRESS = [
    ["shown", "settings-progress-shown"],
    ["minimized", "settings-progress-minimized"],
    ["hidden", "settings-progress-hidden"],
  ] as const;
  let addPrefs = $state<AddPrefs>({ progress_window: "shown", start_at_once: true, browser_at_once: false });
  onMount(() => {
    api.addPrefs().then(
      (p) => (addPrefs = p),
      () => {},
    );
  });
  async function setAddPrefs(next: AddPrefs) {
    const before = addPrefs;
    addPrefs = next;
    await api.setAddPrefs(next).catch(() => (addPrefs = before));
  }

  // The sounds (M5.5): at first only the finished download's.
  const SOUNDS = [
    ["done", "settings-sound-done"],
    ["failed", "settings-sound-failed"],
    ["queue", "settings-sound-queue"],
  ] as const;
  let sounds = $state<Sounds>({ done: true, failed: false, queue: false });
  onMount(() => {
    api.soundPrefs().then(
      (s) => (sounds = s),
      () => {},
    );
  });
  async function setSound(key: "done" | "failed" | "queue", on: boolean) {
    const before = sounds;
    sounds = { ...sounds, [key]: on };
    await api.setSoundPrefs(sounds).catch(() => (sounds = before));
  }

  async function setCloseQuits(quits: boolean) {
    const before = closeQuits;
    closeQuits = quits;
    await api.setCloseQuits(quits).catch(() => (closeQuits = before));
  }

  onMount(() => {
    api.browserIntegration().then(
      (last) => (integration ??= last),
      () => {},
    );
  });
  async function fixIntegration() {
    if (fixing) return;
    fixing = true;
    try {
      integration = await api.fixBrowserIntegration();
      integrationProblem = null;
    } catch (e) {
      integrationProblem = String(e);
    } finally {
      fixing = false;
    }
  }

  // Launching at login is the system's own setting (a login item, a
  // registry value or an autostart file), read from it and changed in it.
  let autostart = $state<boolean | null>(null);
  let autostartProblem = $state<string | null>(null);
  onMount(() => {
    isEnabled().then((on) => (autostart = on), (e) => (autostartProblem = String(e)));
  });
  async function setAutostart(box: HTMLInputElement) {
    autostartProblem = null;
    try {
      await (box.checked ? enable() : disable());
    } catch (e) {
      autostartProblem = `${locale.t("settings-autostart-failed")} ${String(e)}`;
    }
    // What the system holds now, whatever happened: a change that failed
    // left the box as clicked (#60).
    autostart = await isEnabled().catch(() => autostart);
    box.checked = autostart === true;
  }

  // The video tools (M6, #149): what is installed, and deleting them.
  let tools = $state<ToolView[]>([]);
  let toolsSaid = $state<{ text: string; problem: boolean } | null>(null);
  onMount(() => {
    void api.videoToolsStatus().then((t) => (tools = t), (e) => (toolsSaid = { text: String(e), problem: true }));
  });
  async function removeTools() {
    const yes = await ask(locale.t("settings-video-tools-remove-question"), {
      title: locale.t("settings-video-tools"),
      kind: "warning",
      okLabel: locale.t("settings-video-tools-remove"),
      cancelLabel: locale.t("confirm-cancel"),
    }).catch(() => false);
    if (!yes) return;
    try {
      await api.videoToolsRemove();
      toolsSaid = { text: locale.t("settings-video-tools-removed"), problem: false };
    } catch (e) {
      toolsSaid = { text: String(e), problem: true };
    }
    tools = await api.videoToolsStatus().catch(() => tools);
  }

  // Applied as soon as it changes, also while the user types (the owner's
  // notes on 1.0): a moment after the last key.
  let limitTimer: ReturnType<typeof setTimeout> | undefined;
  function setLimit() {
    clearTimeout(limitTimer);
    const n = Number(limitValue);
    const limit = Math.round(n * (limitUnit === "mib" ? MIB : KIB));
    if (Number.isFinite(n) && n > 0 && limit !== settings.speed_limit) change({ speed_limit: limit });
  }
  function limitTyped() {
    clearTimeout(limitTimer);
    limitTimer = setTimeout(setLimit, 400);
  }
</script>

<section class="page" aria-labelledby="settings-title">
  <header>
    <h1 id="settings-title">{locale.t("settings-title")}</h1>
    <p class="hint">{locale.t("settings-applied")}</p>
  </header>

  <fieldset>
    <legend>{locale.t("settings-language")}</legend>
    <div class="choices">
      {#each [["ar", "العربية"], ["en", "English"]] as [value, label] (value)}
        <label class="choice">
          <input type="radio" name="language" {value} checked={settings.language === value} onchange={() => change({ language: value as SettingsChange["language"] })} />
          <span lang={value} dir={value === "ar" ? "rtl" : "ltr"}>{label}</span>
        </label>
      {/each}
    </div>
    <p class="hint">{locale.t("settings-language-hint")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-theme")}</legend>
    <div class="choices">
      {#each ["system", "light", "dark"] as value (value)}
        <label class="choice">
          <input type="radio" name="theme" {value} checked={settings.theme === value} onchange={() => change({ theme: value as SettingsChange["theme"] })} />
          <span>{locale.t(`settings-theme-${value}`)}</span>
        </label>
      {/each}
    </div>
    <p class="hint">{locale.t("settings-theme-hint")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-digits")}</legend>
    <div class="choices">
      {#each [["latn", "123"], ["arab", "١٢٣"]] as [value, label] (value)}
        <label class="choice">
          <input type="radio" name="digits" {value} checked={settings.digits === value} onchange={() => change({ digits: value as SettingsChange["digits"] })} />
          <span class="num">{label}</span>
        </label>
      {/each}
    </div>
    <p class="hint">{locale.t("settings-digits-hint")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-folder")}</legend>
    <div class="row">
      <div class="value"><Icon name="folder" /><bdi dir="ltr">{settings.download_dir ?? settings.system_download_dir}</bdi></div>
      <button class="btn" onclick={onchoosefolder}>{locale.t("settings-folder-choose")}</button>
      {#if settings.download_dir !== null}
        <button class="btn quiet" onclick={() => change({ download_dir: null })}>{locale.t("settings-folder-default")}</button>
      {/if}
    </div>
    <p class="hint">{locale.t("settings-folder-hint")}</p>
  </fieldset>

  <fieldset>
    <legend><label for="connections">{locale.t("settings-connections")}</label></legend>
    <input
      id="connections"
      class="number num"
      type="number"
      min="1"
      max={settings.max_connections}
      value={settings.connections}
      onchange={setConnections}
      aria-describedby="connections-hint"
    />
    <p class="hint" id="connections-hint">
      {locale.t("settings-connections-hint", {
        default: num(locale.lang, locale.digits, 8, 0),
        max: num(locale.lang, locale.digits, settings.max_connections, 0),
      })}
    </p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("sites-title")}</legend>
    <SiteLimitsSetting list={settings.site_connections} onsave={(list) => change({ site_connections: list })} />
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-limit")}</legend>
    <div class="choices">
      <label class="choice">
        <input type="radio" name="limit" checked={settings.speed_limit === null} onchange={() => change({ speed_limit: null })} />
        <span>{locale.t("settings-limit-none")}</span>
      </label>
      <label class="choice">
        <input type="radio" name="limit" checked={settings.speed_limit !== null} onchange={() => { if (!limitValue) limitValue = "1"; setLimit(); }} />
        <span>{locale.t("settings-limit-set")}</span>
      </label>
      <input
        class="number num"
        type="number"
        min="1"
        aria-label={locale.t("settings-limit-value")}
        bind:value={limitValue}
        disabled={settings.speed_limit === null}
        oninput={limitTyped}
        onchange={setLimit}
      />
      <select aria-label={locale.t("settings-limit-unit")} bind:value={limitUnit} disabled={settings.speed_limit === null} onchange={setLimit}>
        <option value="kib">{locale.t("unit-kib-per-second")}</option>
        <option value="mib">{locale.t("unit-mib-per-second")}</option>
      </select>
    </div>
    <p class="hint">{locale.t("settings-limit-hint")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-notifications")}</legend>
    <label class="choice">
      <input type="checkbox" checked={settings.notifications} onchange={(e) => change({ notifications: e.currentTarget.checked })} />
      <span>{locale.t("settings-notifications-label")}</span>
    </label>
    <CompleteSetting />
  </fieldset>

  <fieldset>
    <legend>{locale.t("update-setting-title")}</legend>
    <UpdateSetting checked={settings.update_check} onchange={(on) => change({ update_check: on })} />
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-closing")}</legend>
    <label class="choice">
      <input type="checkbox" checked={closeQuits || !hasTray} disabled={!hasTray} onchange={(e) => void setCloseQuits(e.currentTarget.checked)} />
      <span>{locale.t("settings-closing-label")}</span>
    </label>
    <p class="hint">{locale.t(hasTray ? "settings-closing-hint" : "settings-closing-no-tray")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("drop-box")}</legend>
    <label class="choice">
      <input type="checkbox" checked={dropBox} onchange={(e) => void setDropBox(e.currentTarget.checked)} />
      <span>{locale.t("settings-drop-box-label")}</span>
    </label>
    <p class="hint">{locale.t("settings-drop-box-hint")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-new-download")}</legend>
    <span class="sub">{locale.t("settings-progress-window")}</span>
    {#each PROGRESS as [value, text] (value)}
      <label class="choice">
        <input type="radio" name="progress-window" checked={addPrefs.progress_window === value} onchange={() => void setAddPrefs({ ...addPrefs, progress_window: value })} />
        <span>{locale.t(text)}</span>
      </label>
    {/each}
    <label class="choice">
      <input type="checkbox" checked={addPrefs.start_at_once} onchange={(e) => void setAddPrefs({ ...addPrefs, start_at_once: e.currentTarget.checked })} />
      <span>{locale.t("settings-start-while-open")}</span>
    </label>
    <p class="hint">{locale.t("settings-start-while-open-hint")}</p>
    <label class="choice">
      <input type="checkbox" checked={addPrefs.browser_at_once} onchange={(e) => void setAddPrefs({ ...addPrefs, browser_at_once: e.currentTarget.checked })} />
      <span>{locale.t("settings-browser-at-once")}</span>
    </label>
    <p class="hint">{locale.t("settings-browser-at-once-hint")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-sounds")}</legend>
    {#each SOUNDS as [key, text] (key)}
      <label class="choice">
        <input type="checkbox" checked={sounds[key]} onchange={(e) => void setSound(key, e.currentTarget.checked)} />
        <span>{locale.t(text)}</span>
        <button class="btn quiet hear" type="button" aria-label={locale.t("settings-sound-try")} title={locale.t("settings-sound-try")} onclick={() => play(key)}>
          <Icon name="play" size={13} />
        </button>
      </label>
    {/each}
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-clipboard")}</legend>
    <label class="choice">
      <input type="checkbox" checked={settings.clipboard} onchange={(e) => change({ clipboard: e.currentTarget.checked })} />
      <span>{locale.t("settings-clipboard-label")}</span>
    </label>
    <p class="hint">{locale.t("settings-clipboard-hint")}</p>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-browser")}</legend>
    <p class="hint">{locale.t("settings-browser-hint")}</p>
    <div class="row">
      <button class="btn" aria-disabled={fixing} onclick={() => void fixIntegration()}>{locale.t("settings-browser-fix")}</button>
    </div>
    <div role="status" aria-live="polite" aria-busy={fixing}>
      {#if fixing}
        <p class="hint">{locale.t("settings-browser-working")}</p>
      {:else if integrationProblem}
        <p class="problem">{integrationProblem}</p>
      {:else if integration?.host_problem}
        <p class="problem">{locale.t(HOST_PROBLEMS[integration.host_problem])}</p>
      {:else if integration}
        <ul class="browsers">
          {#each integration.browsers as b (b.browser)}
            <li class={b.error ? "problem" : "done"}>
              {b.error
                ? locale.t("settings-browser-failed", { browser: BROWSERS[b.browser] ?? b.browser, error: b.error })
                : locale.t("settings-browser-ok", { browser: BROWSERS[b.browser] ?? b.browser })}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-video-tools")}</legend>
    <p class="hint">{locale.t("settings-video-tools-hint")}</p>
    {#if tools.length > 0}
      <table class="tools">
        <thead>
          <tr><th scope="col">{locale.t("video-tool")}</th><th scope="col">{locale.t("video-version")}</th><th scope="col">{locale.t("video-license")}</th><th scope="col">{locale.t("progress-status")}</th></tr>
        </thead>
        <tbody>
          {#each tools as t (t.tool)}
            <tr>
              <td>{toolName(t.tool)}</td>
              <td>{versionOf(t)}</td>
              <td>{licenseOf((key, args) => locale.t(key, args), t.tool)}</td>
              <td class:ready={t.installed}>{locale.t(t.installed ? "settings-video-tools-ready" : "settings-video-tools-missing")}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
    {#if tools.some((t) => t.installed)}
      <div class="row">
        <button class="btn" onclick={() => void removeTools()}><Icon name="trash" />{locale.t("settings-video-tools-remove")}</button>
      </div>
    {/if}
    {#if toolsSaid}<p class={toolsSaid.problem ? "problem" : "done"} role="status">{toolsSaid.text}</p>{/if}
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-autostart")}</legend>
    <label class="choice">
      <input type="checkbox" checked={autostart === true} disabled={autostart === null} onchange={(e) => void setAutostart(e.currentTarget)} />
      <span>{locale.t("settings-autostart-label")}</span>
    </label>
    {#if autostartProblem}<p class="hint problem" role="alert">{autostartProblem}</p>{/if}
  </fieldset>

  <fieldset>
    <legend>{locale.t("categories-title")}</legend>
    <p class="hint">{locale.t("categories-hint")}</p>
    <div class="row">
      <button class="btn" onclick={oncategories}><Icon name="tag" />{locale.t("settings-edit-categories")}</button>
    </div>
  </fieldset>

  <fieldset>
    <legend>{locale.t("settings-queues")}</legend>
    <div class="row">
      <button class="btn" onclick={onexport}><Icon name="down" />{locale.t("settings-export")}</button>
      <button class="btn" onclick={onimport}><Icon name="open" />{locale.t("settings-import")}</button>
      <label class="check"><input type="checkbox" bind:checked={keepFolders} />{locale.t("settings-import-keep-folders")}</label>
    </div>
    <p class="hint warn"><Icon name="alert" />{locale.t("settings-export-warning")}</p>
    {#if message}
      <p class={message.problem ? "problem" : "done"} role={message.problem ? "alert" : "status"}>{message.text}</p>
    {/if}
  </fieldset>
</section>

<style>
  .sub {
    font-size: 12.5px;
    color: var(--muted);
  }
  .hear {
    margin-inline-start: 4px;
    padding: 2px 6px;
  }
  .page {
    overflow: auto;
    padding: 20px 28px 32px;
    max-width: 760px;
  }
  header {
    margin-bottom: 8px;
  }
  h1 {
    font-size: 20px;
    margin: 0;
  }
  fieldset {
    border: 0;
    border-bottom: 1px solid var(--line);
    margin: 0;
    padding: 14px 0;
    min-width: 0;
  }
  legend {
    font-weight: 600;
    padding: 0;
    margin-bottom: 8px;
  }
  .choices,
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .choice {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 12px;
    cursor: pointer;
  }
  .choice:has(input:checked) {
    border-color: var(--accent);
    background: var(--sel);
  }
  .choice:has(input:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .choice input {
    margin: 0;
    accent-color: var(--accent);
  }
  .check {
    display: inline-flex;
    gap: 6px;
    align-items: center;
  }
  .value {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    background: var(--sunken);
    border-radius: 8px;
    padding: 6px 10px;
    min-width: 0;
    flex: 1;
    overflow: hidden;
  }
  .value bdi {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .number {
    width: 7em;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 8px;
    background: var(--surface);
  }
  select {
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 8px;
    background: var(--surface);
  }
  .hint {
    color: var(--muted);
    font-size: 12.5px;
    margin: 6px 0 0;
  }
  .warn {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    color: var(--amber);
  }
  .browsers {
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
  }
  .problem,
  .done {
    border-radius: 8px;
    padding: 8px 12px;
    margin: 10px 0 0;
  }
  .problem {
    background: var(--red-bg);
    color: var(--red);
  }
  .done {
    background: var(--green-bg);
    color: var(--green);
  }
  .tools {
    margin: 10px 0 0;
    border-collapse: collapse;
    font-size: 13px;
  }
  .tools th,
  .tools td {
    padding: 5px 14px 5px 0;
    text-align: start;
    border-bottom: 1px solid var(--line);
  }
  .tools th {
    color: var(--muted);
    font-weight: 600;
  }
  .tools .ready {
    color: var(--green);
  }
</style>
