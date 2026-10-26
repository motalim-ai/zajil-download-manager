<script lang="ts">
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { locale } from "../i18n.svelte";
  import { BUILT_IN, categoryName } from "../categoryName";
  import type { CategoryView } from "../bindings/CategoryView";

  let {
    categories,
    onsave,
    onchoosefolder,
    ontry,
    problem,
  }: {
    categories: CategoryView[];
    /** Saves the whole list; resolves to an error message, or null. */
    onsave: (categories: CategoryView[]) => Promise<string | null>;
    /** Asks for a folder; resolves to it, or null if the user cancelled. */
    onchoosefolder: () => Promise<string | null>;
    /** The id of the category a file name (and site) would go to. */
    ontry: (categories: CategoryView[], name: string) => Promise<string | null>;
    problem: string | null;
  } = $props();

  let selectedId = $state<string | null>(null);
  const selected = $derived(categories.find((c) => c.id === selectedId) ?? categories[0] ?? null);
  let error = $state<string | null>(null);
  let newExtension = $state("");
  let newSite = $state("");
  let tryName = $state("");
  let tried = $state<{ name: string; id: string | null } | null>(null);

  const nameOf = (c: CategoryView) => categoryName(c.id, c.name, (k) => locale.t(k));

  async function save(next: CategoryView[]) {
    error = await onsave(next);
  }

  function edit(patch: Partial<CategoryView>) {
    if (!selected) return;
    const id = selected.id;
    void save(categories.map((c) => (c.id === id ? { ...c, ...patch } : c)));
  }

  function addTo(field: "extensions" | "sites", raw: string) {
    if (!selected) return;
    const value = raw.trim().toLowerCase().replace(field === "extensions" ? /^\./ : /^$/, "");
    if (!value || selected[field].includes(value)) return;
    edit({ [field]: [...selected[field], value] });
  }

  function removeFrom(field: "extensions" | "sites", value: string) {
    if (!selected) return;
    edit({ [field]: selected[field].filter((v) => v !== value) });
  }

  async function chooseFolder() {
    const folder = await onchoosefolder();
    if (folder !== null) edit({ folder });
  }

  async function addCategory() {
    // A free id: `custom-1`, `custom-2`…
    let n = 1;
    while (categories.some((c) => c.id === `custom-${n}`)) n += 1;
    const id = `custom-${n}`;
    const name = `${locale.t("categories-new")} ${n}`;
    await save([...categories, { id, name, extensions: [], sites: [], folder: null }]);
    if (!error) selectedId = id;
  }

  async function removeCategory() {
    if (!selected) return;
    const id = selected.id;
    await save(categories.filter((c) => c.id !== id));
    if (!error) selectedId = null;
  }

  async function tryIt() {
    const name = tryName.trim();
    if (!name) {
      tried = null;
      return;
    }
    tried = { name, id: await ontry(categories, name) };
  }

  function onEnter(event: KeyboardEvent, then: () => void) {
    if (event.key === "Enter") {
      event.preventDefault();
      then();
    }
  }
</script>

<section class="page" aria-labelledby="categories-title">
  <header>
    <h1 id="categories-title">{locale.t("categories-title")}</h1>
    <p class="hint">{locale.t("categories-hint")}</p>
  </header>
  {#if problem || error}<p class="problem" role="alert"><Icon name="alert" />{error ?? problem}</p>{/if}

  <div class="split">
    <div class="list">
      <ul aria-label={locale.t("categories-title")}>
        {#each categories as c (c.id)}
          <li>
            <button class="item" aria-current={selected?.id === c.id ? "true" : undefined} onclick={() => (selectedId = c.id)}>
              <Icon name="tag" /><bdi>{nameOf(c)}</bdi>
            </button>
          </li>
        {/each}
      </ul>
      <button class="btn" onclick={() => void addCategory()}><Icon name="plus" />{locale.t("categories-new")}</button>
    </div>

    {#if selected}
      {@const c = selected}
      <div class="editor">
        <label class="label" for="category-name">{locale.t("categories-name")}</label>
        <input
          id="category-name"
          class="text"
          value={nameOf(c)}
          onchange={(e) => {
            const value = (e.currentTarget as HTMLInputElement).value.trim();
            // A built-in category keeps its translated name until renamed.
            const name = BUILT_IN.includes(c.id) && value === locale.t(`category-${c.id}`) ? null : value || null;
            edit({ name });
          }}
        />

        <span class="label" id="extensions-label">{locale.t("categories-extensions")}</span>
        <ul class="chips" aria-labelledby="extensions-label">
          {#each c.extensions as ext (ext)}
            <li class="chip">
              <bdi dir="ltr">.{ext}</bdi>
              <button class="remove" aria-label={locale.t("categories-remove", { value: ext })} onclick={() => removeFrom("extensions", ext)}><Icon name="x" /></button>
            </li>
          {/each}
          <li>
            <input
              class="chip-input"
              dir="ltr"
              aria-label={locale.t("categories-add-extension")}
              aria-describedby="extensions-hint"
              placeholder={locale.t("categories-add-extension")}
              bind:value={newExtension}
              onkeydown={(e) => onEnter(e, () => { addTo("extensions", newExtension); newExtension = ""; })}
            />
          </li>
        </ul>
        <p class="hint" id="extensions-hint">{locale.t("categories-extensions-hint")}</p>

        <span class="label" id="sites-label">{locale.t("categories-sites")}</span>
        <ul class="chips" aria-labelledby="sites-label">
          {#each c.sites as site (site)}
            <li class="chip">
              <bdi dir="ltr">{site}</bdi>
              <button class="remove" aria-label={locale.t("categories-remove", { value: site })} onclick={() => removeFrom("sites", site)}><Icon name="x" /></button>
            </li>
          {/each}
          <li>
            <input
              class="chip-input"
              dir="ltr"
              aria-label={locale.t("categories-add-site")}
              aria-describedby="sites-hint"
              placeholder="example.com"
              bind:value={newSite}
              onkeydown={(e) => onEnter(e, () => { addTo("sites", newSite); newSite = ""; })}
            />
          </li>
        </ul>
        <p class="hint" id="sites-hint">{locale.t("categories-sites-hint")}</p>

        <span class="label">{locale.t("categories-folder")}</span>
        <div class="row">
          <div class="value"><Icon name="folder" />{#if c.folder}<bdi dir="ltr">{c.folder}</bdi>{:else}<span class="muted">{locale.t("categories-folder-none")}</span>{/if}</div>
          <button class="btn" onclick={() => void chooseFolder()}>{locale.t("settings-folder-choose")}</button>
          {#if c.folder}<button class="btn quiet" onclick={() => edit({ folder: null })}>{locale.t("categories-folder-clear")}</button>{/if}
        </div>

        <div class="danger">
          <button class="btn quiet" onclick={() => void removeCategory()}><Icon name="trash" />{locale.t("categories-delete")}</button>
        </div>
      </div>
    {/if}
  </div>

  <div class="try">
    <label class="label" for="try-name">{locale.t("categories-try")}</label>
    <div class="row">
      <input id="try-name" class="text" dir="auto" placeholder="ملف.zip" aria-describedby="try-hint" bind:value={tryName} onkeydown={(e) => onEnter(e, () => void tryIt())} />
      <button class="btn" onclick={() => void tryIt()}>{locale.t("categories-try")}</button>
    </div>
    <p class="hint" id="try-hint">{locale.t("categories-try-hint")}</p>
    {#if tried}
      {@const found = categories.find((c) => c.id === tried?.id)}
      <p class="result" role="status">
        <FileName name={tried.name} /> <span aria-hidden="true">{locale.dir === "rtl" ? "←" : "→"}</span>
        <b>{found ? nameOf(found) : locale.t("category-none")}</b>
      </p>
    {/if}
  </div>
</section>

<style>
  .page {
    overflow: auto;
    padding: 20px 28px 32px;
  }
  h1 {
    font-size: 20px;
    margin: 0;
  }
  .split {
    display: grid;
    grid-template-columns: 220px 1fr;
    gap: 20px;
    margin-top: 14px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    border: 0;
    background: none;
    padding: 6px 10px;
    border-radius: 7px;
    text-align: start;
    cursor: pointer;
  }
  .item[aria-current="true"] {
    background: var(--sel);
    font-weight: 600;
  }
  .editor {
    display: grid;
    gap: 6px;
    align-content: start;
    min-width: 0;
  }
  .label {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--muted);
    margin-top: 8px;
  }
  .text {
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 6px 10px;
    background: var(--surface);
    min-width: 0;
    flex: 1;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 6px;
    background: var(--sunken);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--sel);
    border-radius: 999px;
    padding: 1px 4px 1px 10px;
  }
  .remove {
    border: 0;
    background: none;
    padding: 2px;
    cursor: pointer;
    color: var(--muted);
    display: inline-flex;
    border-radius: 999px;
  }
  .chip-input {
    border: 0;
    background: none;
    min-width: 8em;
    padding: 2px 4px;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
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
  .muted {
    color: var(--muted);
  }
  .danger {
    margin-top: 12px;
    color: var(--red);
  }
  .try {
    margin-top: 22px;
    padding-top: 14px;
    border-top: 1px solid var(--line);
    max-width: 560px;
  }
  .hint {
    color: var(--muted);
    font-size: 12.5px;
    margin: 2px 0 0;
  }
  .result {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 10px 0 0;
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: center;
    background: var(--red-bg);
    color: var(--red);
    border-radius: 8px;
    padding: 8px 12px;
    margin: 10px 0 0;
  }
</style>
