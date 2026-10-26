<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import { checkList, MAX_CONNECTIONS, type SiteRow } from "../siteList";

  // The per-site connection limits (#165, owner's decision): no limit by
  // default; a list of exceptions, site = the most connections at once,
  // shared by every download on it. Edited here, saved with «Save list».
  let { list, onsave }: { list: SiteRow[]; onsave: (list: SiteRow[]) => void } = $props();

  let rows = $state<SiteRow[]>([]);
  let problem = $state<string | null>(null);
  // The saved list, as last given: rows follow it until edited.
  let given = "";
  $effect(() => {
    const now = JSON.stringify(list);
    if (now !== given) {
      given = now;
      rows = list.map((r) => ({ ...r }));
      problem = null;
    }
  });
  const dirty = $derived(JSON.stringify(rows) !== JSON.stringify(list));

  function add() {
    rows = [...rows, { site: "", connections: 2 }];
  }

  function save() {
    const checked = checkList(rows);
    if ("problem" in checked) {
      const { key, site } = checked.problem;
      problem = site ? `${locale.t(key)} ${site}` : locale.t(key);
      return;
    }
    problem = null;
    onsave(checked.list);
  }
</script>

<p class="hint">{locale.t("sites-hint")}</p>
{#if rows.length === 0}
  <p class="hint">{locale.t("sites-empty")}</p>
{:else}
  <div class="sites" role="group" aria-label={locale.t("sites-title")}>
    <span class="head">{locale.t("sites-site")}</span>
    <span class="head">{locale.t("sites-connections")}</span>
    <span></span>
    {#each rows as row, i (i)}
      <input class="site" dir="ltr" bind:value={row.site} placeholder="example.com" aria-label={locale.t("sites-site")} />
      <input class="number" type="number" min="1" max={MAX_CONNECTIONS} bind:value={row.connections} aria-label={locale.t("sites-connections")} />
      <button class="btn icon quiet" title={locale.t("sites-remove")} aria-label={locale.t("sites-remove")} onclick={() => (rows = rows.filter((_, j) => j !== i))}>
        <Icon name="trash" />
      </button>
    {/each}
  </div>
{/if}
<div class="row">
  <button class="btn" onclick={add}><Icon name="plus" />{locale.t("sites-add")}</button>
  <button class="btn primary" disabled={!dirty} onclick={save}><Icon name="check" />{locale.t("sites-save")}</button>
</div>
{#if problem}<p class="problem" role="alert"><Icon name="alert" />{problem}</p>{/if}

<style>
  .hint {
    font-size: 12.5px;
    color: var(--muted);
    margin: 0 0 8px;
  }
  .sites {
    display: grid;
    grid-template-columns: minmax(180px, 320px) 90px auto;
    gap: 6px 10px;
    align-items: center;
    margin-bottom: 8px;
  }
  .head {
    font-size: 12.5px;
    color: var(--muted);
  }
  input {
    font: inherit;
    color: var(--ink);
    background: var(--sunken);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 8px;
    min-height: 32px;
    box-sizing: border-box;
    min-width: 0;
  }
  .site {
    text-align: left;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .problem {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--red);
    margin: 8px 0 0;
  }
</style>
