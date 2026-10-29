<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { nameDir } from "../fname";
  import { categoryIcon } from "./Tree.svelte";
  import { locale } from "../i18n.svelte";
  import { categoryName as nameOf } from "../categoryName";
  import { probeSentence, sizeText } from "../probeText";
  import { chosenFolder, savePath, typedPath } from "../addFolder";
  import { fitWindow } from "../fitWindow";
  import type { ProbeView } from "../bindings/ProbeView";
  import type { AddRequest } from "../bindings/AddRequest";
  import type { QueueView } from "../bindings/QueueView";
  import type { CategoryView } from "../bindings/CategoryView";

  // «Download file info» (M5.5, the design of #83, window 3): what the
  // server said of the file, where it goes, and when. Small, as IDM's
  // (the owner's notes on 1.0): the file's kind and size in one line, the
  // path in one «Save as» box, the three buttons together. A window of its own;
  // the UI test scene draws it in the main one. The file name and the
  // category can be changed once zajil-core keeps them (#103), the
  // description once it has one (#95).
  let {
    url,
    probe,
    guess = null,
    error,
    busy,
    queues,
    categories,
    onadd,
    oncancel,
    onchoosefolder,
    onnewcategory,
    started = false,
    connections = 8,
    maxConnections = 32,
  }: {
    url: string;
    probe: ProbeView | null;
    /// What the link's address says, before the server answers: every row
    /// is drawn with it, and the answer fills them in, so the window never
    /// grows (the owner's notes: it opened small, then grew).
    guess?: ProbeView | null;
    error: string | null;
    /// An add is on its way: the buttons wait (review of #48: two clicks
    /// added the download twice).
    busy: boolean;
    queues: QueueView[];
    categories: CategoryView[];
    /// `remember`: the category whose folder the chosen one becomes.
    onadd: (request: AddRequest, remember: string | null) => void;
    oncancel: () => void;
    onchoosefolder: () => Promise<string | null>;
    onnewcategory: () => void;
    /// The download started already, in the background (the setting
    /// «Start downloading while the window is open»): it moves to the name
    /// and folder chosen here once the user says.
    started?: boolean;
    /// The program's connections for a download, shown at first.
    connections?: number;
    maxConnections?: number;
  } = $props();

  // The queue it goes to, and its own connections (the owner's request, as
  // in other download managers); `null`: the program's.
  let queue = $state("main");
  let ownConnections = $state<number | null>(null);
  function connectionsTyped(box: HTMLInputElement) {
    const n = Math.round(Number(box.value));
    if (Number.isFinite(n) && n >= 1 && n <= maxConnections) ownConnections = n === connections ? null : n;
    else box.value = String(ownConnections ?? connections);
  }

  // A folder the user chose by hand; `null`: the picked category's.
  let folder = $state<string | null>(null);
  // The category the user picked (the detected one at first). Its folder
  // becomes the download's; the category itself is kept once zajil-core
  // keeps one for an item (#103).
  let picked = $state<string | null | undefined>(undefined);
  // The server's answer, else what the address says.
  const shown = $derived(probe ?? guess);
  const category = $derived(picked === undefined ? (shown?.category ?? null) : picked);
  const where = $derived(
    shown
      ? chosenFolder({
          categories,
          detected: shown.category,
          picked: category,
          byHand: folder,
          detectedFolder: shown.folder,
          downloadFolder: shown.download_folder,
        })
      : { shown: "", request: null },
  );
  let remember = $state(false);
  let later = $state(false);
  let main: HTMLElement | undefined = $state();

  // The focus goes to the main button once the window has its rows:
  // «Start download», or «Cancel» once the server says the link is a web
  // page, unless the user is typing in a box by then. Without scrolling
  // (the owner's notes on the 1.0 draft, the rows went off the top).
  let focused = false;
  const page = $derived(probe?.page === true);
  // The buttons wait for an add on its way, and for a link the server
  // refused; before the server answers they add with what the address says.
  const blocked = $derived(busy || (error !== null && !probe));
  $effect(() => {
    if (!shown || busy) return;
    const typing = document.activeElement instanceof HTMLInputElement || document.activeElement instanceof HTMLSelectElement;
    if (focused && (!page || typing)) return;
    focused = true;
    main?.querySelector<HTMLButtonElement>("[data-autofocus]")?.focus({ preventScroll: true });
  });

  // The user's own name, else the translated built-in one (as the sidebar).
  const categoryName = (id: string) => nameOf(id, categories.find((c) => c.id === id)?.name, (k) => locale.t(k));
  const extension = $derived((/\.([^.]+)$/.exec(shown?.name ?? "")?.[1] ?? "").toUpperCase());

  // The name and the description as the user leaves them (#103, #95).
  let name = $state("");
  let description = $state("");
  // The user's own name stays when the server answers.
  let ownName = false;
  $effect(() => {
    if (shown && !ownName) name = shown.name;
  });
  // «Save as»: one box with the whole path, the folder and the name (as
  // IDM's). Typed, its last part is the name and the rest the folder.
  const fullPath = $derived(savePath(where.shown, name));
  // The box is being edited: until then the path is drawn with the name as
  // FileName draws it (a box orders a mixed name as one paragraph, the
  // rendering the owner rejected in M4).
  let editing = $state(false);
  let typed = $state("");
  function startEditing() {
    typed = fullPath;
    editing = true;
  }
  /** What the user typed in «Save as», as a folder and a name. */
  function takeTyped() {
    editing = false;
    const said = typedPath(typed, where.shown);
    // An emptied name means the server's.
    if (said.name && said.name !== name) ownName = true;
    if (!said.name) ownName = false;
    name = said.name || (shown?.name ?? "");
    if (said.folder !== null) folder = said.folder;
  }
  function add(start: boolean, into = queue) {
    if (editing) takeTyped();
    later = false;
    const own = name.trim();
    const request = {
      url,
      name: own && own !== shown?.name ? own : null,
      // Kept only when the user picked one (#103); else by name or site.
      category: picked === undefined ? null : category,
      description: description.trim() || null,
      queue: into,
      folder: where.request,
      start,
      limit: null,
      connections: ownConnections,
    };
    if (!busy) onadd(request, remember && folder !== null ? category : null);
  }
  async function change() {
    const chosen = await onchoosefolder();
    if (chosen !== null) folder = chosen;
  }
  async function openLater() {
    later = !later;
    if (later) {
      await tick();
      main?.querySelector<HTMLButtonElement>(".queues [role='menuitem']")?.focus();
    }
  }
  function onQueuesKey(event: KeyboardEvent) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const items = [...(main?.querySelectorAll<HTMLButtonElement>(".queues [role='menuitem']") ?? [])];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    items[(at + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
  }
  // Enter starts the download (as IDM's), except on a button, which does
  // its own, and on a web page; Escape closes the queues, then cancels.
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      if (event.target instanceof HTMLButtonElement || later || blocked || page || event.isComposing) return;
      event.preventDefault();
      add(true);
      return;
    }
    if (event.key !== "Escape") return;
    event.preventDefault();
    if (later) {
      later = false;
      document.getElementById("add-later-more")?.focus();
    } else oncancel();
  }
</script>

<svelte:window {onkeydown} />
<svelte:document onpointerdown={(e) => later && !(e.target as Element).closest?.(".split") && (later = false)} />

<main class="info" bind:this={main} use:fitWindow aria-labelledby="add-title">
  <h1 id="add-title" class="visually-hidden">{locale.t("info-title")}</h1>
  <!-- What the server said, in one line (as IDM's). -->
  <div class="about" aria-live="polite" aria-label={locale.t("add-about-file")}>
    {#if error}
      <p class="problem" role="alert"><Icon name="alert" size={15} /><span>{error}</span></p>
    {:else if page}
      <p class="page" role="alert"><Icon name="circle-alert" size={15} /><span>{locale.t("add-web-page")}</span></p>
    {:else if probe}
      <p class="known" title={probeSentence(locale.t, locale.lang, locale.digits, probe)}>
        <span class="type" aria-hidden="true"><Icon name={categoryIcon(probe.category)} size={15} /></span>
        <strong>{sizeText(locale.t, locale.lang, locale.digits, probe)}</strong>
        <span class="kind">{probe.category ? categoryName(probe.category) : locale.t("category-none")} {extension}</span>
        <span class="resume" class:no={!probe.resumable}>
          <Icon name={probe.resumable ? "check" : "circle-alert"} size={14} />{locale.t(probe.resumable ? "add-resumable" : "add-not-resumable")}
        </span>
      </p>
    {:else}
      <!-- The same line while the server is asked: the size waits in its
           place, and nothing moves when it comes. -->
      <p class="known" role="status">
        <span class="type" aria-hidden="true"><Icon name={categoryIcon(shown?.category ?? null)} size={15} /></span>
        <strong class="pending" aria-hidden="true">…</strong>
        <span class="kind">{shown?.category ? categoryName(shown.category) : locale.t("category-none")} {extension}</span>
        <span class="waiting">{locale.t("add-checking")}</span>
      </p>
    {/if}
  </div>

  <div class="fields">
    <span class="label">{locale.t("add-url")}</span>
    <div class="value"><bdi dir="ltr" class="text">{url}</bdi></div>

    <!-- Every row from the first frame (the owner's notes): the server's
         answer fills them in, it adds none. -->
    <span class="label">{locale.t("add-category")}</span>
    <div class="with">
      <label class="value pick">
        <Icon name={categoryIcon(category)} size={15} />
        <select
          aria-label={locale.t("add-category")}
          value={category ?? ""}
          onchange={(e) => (picked = e.currentTarget.value || null)}
        >
          {#each categories as c (c.id)}
            <option value={c.id}>
              {#if c.id === shown?.category && shown.category_site}
                {locale.t("add-category-site", { category: categoryName(c.id), site: shown.category_site })}
              {:else if c.id === shown?.category && shown.category_extension}
                {locale.t("add-category-extension", { category: categoryName(c.id), extension: shown.category_extension })}
              {:else}
                {categoryName(c.id)}
              {/if}
            </option>
          {/each}
          <option value="">{locale.t("category-none")}</option>
        </select>
      </label>
      <button class="btn square" title={locale.t("add-new-category")} aria-label={locale.t("add-new-category")} onclick={onnewcategory}>
        <Icon name="circle-plus" size={15} />
      </button>
    </div>

    <label class="label" for="add-name">{locale.t("add-save-as")}</label>
    <div class="with">
      <!-- The whole path: the folder, then the name it is saved under
           (the server's, or the user's own, #103). -->
      <div class="namebox">
        <input
          id="add-name"
          class="value name"
          class:quiet={!editing}
          dir="ltr"
          autocomplete="off"
          spellcheck="false"
          value={editing ? typed : fullPath}
          oninput={(e) => (typed = e.currentTarget.value)}
          onfocus={startEditing}
          onblur={takeTyped}
        />
        <!-- Only drawn: the box holds the path for the keyboard and
             screen readers, and clicks go through to it. -->
        {#if !editing}
          <span class="value name shown" dir="ltr" aria-hidden="true">
            <bdi class="dir">{fullPath.slice(0, fullPath.length - name.length)}</bdi><span class="file" dir={nameDir(name)}><FileName {name} /></span>
          </span>
        {/if}
      </div>
      <button class="btn square" title={locale.t("add-choose-folder")} aria-label={locale.t("add-choose-folder")} onclick={() => void change()}>...</button>
    </div>

    <!-- Off for a web page, in their place. -->
    <label class="label" for="add-queue">{locale.t("add-queue")}</label>
    <div class="with">
      <label class="value pick">
        <Icon name="list-ordered" size={15} />
        <select id="add-queue" bind:value={queue} disabled={page}>
          {#each queues as q (q.name)}
            <option value={q.name}>{q.name === "main" ? locale.t("queue-main") : q.name}</option>
          {/each}
        </select>
      </label>
      <label class="conns">
        <span class="label">{locale.t("add-connections")}</span>
        <input
          class="value num count"
          type="number"
          min="1"
          max={maxConnections}
          value={ownConnections ?? connections}
          disabled={page}
          onchange={(e) => connectionsTyped(e.currentTarget)}
        />
      </label>
    </div>
    <label class="label" for="add-description">{locale.t("add-description")}</label>
    <input
      id="add-description"
      class="value name"
      dir="auto"
      maxlength="1000"
      placeholder={locale.t("add-description-hint")}
      disabled={page}
      bind:value={description}
    />
  </div>

  <div class="actions">
    {#if shown && category && folder !== null}
      <label class="check">
        <input type="checkbox" bind:checked={remember} />
        {locale.t("add-remember", { category: categoryName(category) })}
      </label>
    {/if}
    <div class="buttons">
      <button class="btn" class:primary={!page} disabled={blocked} onclick={() => add(true)} data-autofocus={page ? undefined : true}>
        <Icon name="down" size={15} />{locale.t("add-start")}
      </button>
      <div class="split">
        <button class="btn" disabled={blocked} onclick={() => add(false)}>
          <Icon name="list-ordered" size={15} />{locale.t("add-later")}
        </button>
        <button
          id="add-later-more"
          class="btn more"
          disabled={blocked}
          aria-label={locale.t("add-later-queue")}
          aria-haspopup="menu"
          aria-expanded={later}
          onclick={() => void openLater()}
        >
          <Icon name="chevron-down" size={14} />
        </button>
        {#if later}
          <div class="queues" role="menu" tabindex="-1" aria-label={locale.t("add-later-queue")} onkeydown={onQueuesKey}>
            {#each queues as q (q.name)}
              <button class="q" role="menuitem" onclick={() => add(false, q.name)}>
                <bdi>{q.name === "main" ? locale.t("queue-main") : q.name}</bdi>
              </button>
            {/each}
          </div>
        {/if}
      </div>
      <button class="btn" class:primary={page} onclick={oncancel} data-autofocus={page ? true : undefined}>{locale.t("add-cancel")}</button>
    </div>
  </div>
</main>

<style>
  /* At its own height: the window is made as tall (fitWindow). */
  .info {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 16px 12px;
    background: var(--surface);
    box-sizing: border-box;
  }
  /* Two lines, whatever it says (the size, a web page, an error): the rows
     under it never move. */
  .about {
    display: flex;
    align-items: center;
    height: 40px;
    min-width: 0;
  }
  .about p {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 0;
    min-height: 22px;
    font-size: 13px;
  }
  .known {
    flex-wrap: wrap;
  }
  .type {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: var(--sel);
    color: var(--accent);
  }
  .kind {
    color: var(--muted);
  }
  .resume {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    color: var(--green);
  }
  .resume.no {
    color: var(--amber);
  }
  .waiting,
  .pending {
    color: var(--muted);
  }
  .problem {
    color: var(--red);
  }
  .problem span,
  .page span {
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .page {
    padding: 4px 8px;
    border-radius: 7px;
    background: var(--amber-bg);
    color: var(--ink);
  }
  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 14px;
    align-content: start;
    align-items: center;
    min-width: 0;
  }
  .label {
    font-size: 13px;
    color: var(--muted);
  }
  .with {
    display: flex;
    gap: 6px;
    min-width: 0;
  }
  .with > :first-child {
    flex: 1;
  }
  .value {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    background: var(--sunken);
    border-radius: 7px;
    padding: 4px 9px;
    min-height: 30px;
    min-width: 0;
    box-sizing: border-box;
    overflow: hidden;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  /* Never narrower than a few letters, whatever the row does (the
     owner's test: the name came out empty). */
  .name {
    min-width: 8ch;
    font: inherit;
    color: var(--ink);
    outline: none;
  }
  .name:focus {
    border-color: var(--accent);
    background: var(--surface);
  }
  .namebox {
    display: grid;
    min-width: 0;
  }
  .namebox > * {
    grid-area: 1 / 1;
  }
  .name.quiet {
    color: transparent;
  }
  /* The folder gives way first, so the name stays in sight. */
  .name.shown {
    pointer-events: none;
    background: transparent;
    border-color: transparent;
    gap: 0;
    white-space: nowrap;
  }
  .shown .dir {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--muted);
  }
  .shown .file {
    flex: 0 0 auto;
    max-width: 85%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name::placeholder {
    color: var(--faint);
  }
  .pick select {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    font: inherit;
    color: var(--ink);
    outline: none;
  }
  .pick:focus-within {
    border-color: var(--accent);
  }
  .conns {
    display: flex;
    gap: 8px;
    align-items: center;
    flex: none;
  }
  .count {
    width: 64px;
    font: inherit;
    color: var(--ink);
  }
  .square {
    padding-inline: 8px;
    min-width: 32px;
    justify-content: center;
  }
  .actions {
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .check {
    margin: 0;
    font-size: 12px;
    color: var(--muted);
    min-width: 0;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--ink);
  }
  .buttons {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .split {
    position: relative;
    display: flex;
  }
  .split > .btn:first-child {
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }
  .more {
    border-start-start-radius: 0;
    border-end-start-radius: 0;
    border-inline-start: 0;
    padding-inline: 7px;
  }
  .queues {
    position: absolute;
    bottom: calc(100% + 4px);
    inset-inline-start: 0;
    min-width: 100%;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
    z-index: 5;
  }
  .q {
    border: 0;
    background: none;
    text-align: start;
    padding: 6px 10px;
    border-radius: 6px;
    font: inherit;
    color: var(--ink);
  }
  .q:hover,
  .q:focus-visible {
    background: var(--sel);
    outline: none;
  }
</style>
