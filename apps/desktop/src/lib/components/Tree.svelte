<script lang="ts" module>
  /** What the table shows (M5.5): every download, a category (or only its
   * unfinished or finished ones), the downloads of no category, or a
   * queue's. */
  export type Filter =
    | { kind: "all" }
    | { kind: "category"; id: string; part?: "unfinished" | "finished" }
    | { kind: "uncategorized" }
    | { kind: "queue"; name: string };

  export type View = "list" | "settings" | "categories";

  /** A category's icon (Lucide), by its built-in id. */
  export function categoryIcon(id: string | null): string {
    return ({ programs: "package", video: "film", music: "music", documents: "file-text", archives: "archive" } as Record<string, string>)[id ?? ""] ?? "file";
  }
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import { num } from "../format";
  import type { Snapshot } from "../bindings/Snapshot";

  let {
    snapshot,
    filter = $bindable(),
    view = $bindable(),
  }: { snapshot: Snapshot; filter: Filter; view: View } = $props();

  type Download = Snapshot["downloads"][number];

  // The categories whose «unfinished» and «finished» show under them.
  let open = $state<Set<string>>(new Set());

  function show(f: Filter) {
    filter = f;
    view = "list";
  }
  function toggle(id: string) {
    const next = new Set(open);
    if (!next.delete(id)) next.add(id);
    open = next;
  }
  const count = (pick: (d: Download) => boolean) => num(locale.lang, locale.digits, snapshot.downloads.filter(pick).length, 0);
  const same = (a: Filter, b: Filter) => JSON.stringify(a) === JSON.stringify(b);
  const current = (f: Filter) => (view === "list" && same(filter, f) ? "page" : undefined);
  const categoryName = (c: Snapshot["categories"][number]) => c.name ?? locale.t(`category-${c.id}`);
  const queueName = (name: string) => (name === "main" ? locale.t("queue-main") : name);
</script>

<nav class="tree" aria-label={locale.t("nav-categories")}>
  <button class="node top" aria-current={current({ kind: "all" })} onclick={() => show({ kind: "all" })}>
    <Icon name="layers" size={16} /><span class="label">{locale.t("nav-all")}</span><span class="n num">{count(() => true)}</span>
  </button>

  <h2>{locale.t("nav-categories")}</h2>
  {#each snapshot.categories as c (c.id)}
    {@const mine = (d: Download) => d.category === c.id}
    {@const expanded = open.has(c.id)}
    <div class="row">
      <button
        class="twist"
        aria-expanded={expanded}
        aria-label={`${locale.t("tree-expand")}: ${categoryName(c)}`}
        onclick={() => toggle(c.id)}
      ><span class="flip"><Icon name={expanded ? "chevron-down" : "chevron-right"} size={14} /></span></button>
      <button class="node" aria-current={current({ kind: "category", id: c.id })} onclick={() => show({ kind: "category", id: c.id })}>
        <Icon name={categoryIcon(c.id)} size={16} /><span class="label">{categoryName(c)}</span><span class="n num">{count(mine)}</span>
      </button>
    </div>
    {#if expanded}
      {#each [{ part: "unfinished", icon: "circle-dashed", key: "tree-unfinished", pick: (d: Download) => mine(d) && d.status !== "done" }, { part: "finished", icon: "circle-check", key: "tree-finished", pick: (d: Download) => mine(d) && d.status === "done" }] as const as sub (sub.part)}
        <button class="node l1" aria-current={current({ kind: "category", id: c.id, part: sub.part })} onclick={() => show({ kind: "category", id: c.id, part: sub.part })}>
          <Icon name={sub.icon} size={15} /><span class="label">{locale.t(sub.key)}</span><span class="n num">{count(sub.pick)}</span>
        </button>
      {/each}
    {/if}
  {/each}
  {#if snapshot.downloads.some((d) => d.category === null)}
    <button class="node l0" aria-current={current({ kind: "uncategorized" })} onclick={() => show({ kind: "uncategorized" })}>
      <Icon name="file" size={16} /><span class="label">{locale.t("category-none")}</span><span class="n num">{count((d) => d.category === null)}</span>
    </button>
  {/if}

  <h2>{locale.t("nav-queues")}</h2>
  {#each snapshot.queues as q (q.name)}
    <button class="node l0" aria-current={current({ kind: "queue", name: q.name })} onclick={() => show({ kind: "queue", name: q.name })}>
      <Icon name={q.window ? "calendar-clock" : "list-ordered"} size={16} />
      <span class="label">
        <bdi>{queueName(q.name)}</bdi>
        {#if q.window}<small><bdi>{locale.t("queue-starts", { time: q.window[0] })}</bdi></small>{/if}
      </span>
      <span class="n num">{count((d) => d.queue === q.name)}</span>
    </button>
  {/each}
</nav>

<style>
  .tree {
    background: var(--side);
    border-inline-end: 1px solid var(--line);
    padding: 10px 8px;
    overflow: auto;
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  h2 {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--faint);
    margin: 12px 8px 2px;
  }
  .row {
    display: flex;
    align-items: center;
  }
  .twist {
    width: 22px;
    height: 26px;
    display: grid;
    place-items: center;
    border: 0;
    background: none;
    color: var(--faint);
    cursor: pointer;
    border-radius: 6px;
    flex: none;
  }
  :global([dir="rtl"]) .flip {
    display: inline-flex;
    transform: scaleX(-1);
  }
  .node {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 6px;
    border-radius: 7px;
    border: 0;
    background: none;
    color: var(--ink);
    text-align: start;
    cursor: pointer;
    flex: 1;
    min-width: 0;
    font: inherit;
  }
  .node:hover {
    background: color-mix(in oklab, var(--sel) 60%, transparent);
  }
  .node[aria-current="page"] {
    background: var(--sel);
    font-weight: 600;
  }
  .node :global(.icon) {
    color: var(--muted);
  }
  .top {
    font-size: 14px;
  }
  .l0 {
    margin-inline-start: 22px;
  }
  .l1 {
    margin-inline-start: 40px;
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  small {
    color: var(--faint);
    font-size: 11px;
    line-height: 1.3;
  }
  .n {
    margin-inline-start: auto;
    color: var(--faint);
    font-size: 12px;
  }
</style>
