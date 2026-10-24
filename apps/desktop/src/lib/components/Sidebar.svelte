<script lang="ts">
  import Icon from "./Icon.svelte";
  import Mark from "./Mark.svelte";
  import { locale } from "../i18n.svelte";
  import { num } from "../format";
  import type { Snapshot } from "../bindings/Snapshot";

  export type Filter = { kind: "all" | "active" | "done" } | { kind: "category"; id: string } | { kind: "queue"; name: string };

  let { snapshot, filter = $bindable() }: { snapshot: Snapshot; filter: Filter } = $props();

  const count = (pick: (d: Snapshot["downloads"][number]) => boolean) =>
    num(locale.lang, locale.digits, snapshot.downloads.filter(pick).length, 0);
  const same = (a: Filter, b: Filter) => JSON.stringify(a) === JSON.stringify(b);
  const categoryName = (c: Snapshot["categories"][number]) => c.name ?? locale.t(`category-${c.id}`);
</script>

<nav class="side" aria-label={locale.t("app-name")}>
  <div class="brand"><Mark size={28} /><span>{locale.t("app-name")}</span></div>
  {#each [{ f: { kind: "all" }, icon: "all", key: "nav-all", pick: () => true }, { f: { kind: "active" }, icon: "down", key: "nav-active", pick: (d: Snapshot["downloads"][number]) => d.status === "active" }, { f: { kind: "done" }, icon: "check", key: "nav-done", pick: (d: Snapshot["downloads"][number]) => d.status === "done" }] as item (item.key)}
    <button class="nav" aria-current={same(filter, item.f as Filter) ? "page" : undefined} onclick={() => (filter = item.f as Filter)}>
      <Icon name={item.icon} /><span class="label">{locale.t(item.key)}</span><span class="count num">{count(item.pick)}</span>
    </button>
  {/each}
  <h2>{locale.t("nav-categories")}</h2>
  {#each snapshot.categories as c (c.id)}
    {@const f: Filter = { kind: "category", id: c.id }}
    <button class="nav" aria-current={same(filter, f) ? "page" : undefined} onclick={() => (filter = f)}>
      <Icon name="tag" /><span class="label">{categoryName(c)}</span><span class="count num">{count((d) => d.category === c.id)}</span>
    </button>
  {/each}
  <h2>{locale.t("nav-queues")}</h2>
  {#each snapshot.queues as q (q.name)}
    {@const f: Filter = { kind: "queue", name: q.name }}
    <button class="nav" aria-current={same(filter, f) ? "page" : undefined} onclick={() => (filter = f)}>
      <Icon name="queue" />
      <span class="label">
        <bdi>{q.name}</bdi>
        {#if q.window}<small><bdi>{locale.t("queue-window", { start: q.window[0], stop: q.window[1] })}</bdi></small>{/if}
      </span>
      <span class="count num">{num(locale.lang, locale.digits, q.active, 0)}/{num(locale.lang, locale.digits, q.max_active, 0)}</span>
    </button>
  {/each}
</nav>

<style>
  .side {
    background: var(--side);
    border-inline-end: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    padding: 14px 10px;
    gap: 2px;
    overflow: auto;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 8px 14px;
    font-weight: 700;
    font-size: 17px;
  }
  h2 {
    font-size: 12px;
    font-weight: 600;
    color: var(--faint);
    margin: 14px 8px 4px;
  }
  .nav {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border-radius: 7px;
    border: 0;
    background: none;
    width: 100%;
    text-align: start;
    cursor: pointer;
    color: var(--ink);
  }
  .nav:hover {
    background: color-mix(in oklab, var(--sel) 60%, transparent);
  }
  .nav[aria-current="page"] {
    background: var(--sel);
    font-weight: 600;
  }
  .nav :global(.icon) {
    color: var(--muted);
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  small {
    color: var(--faint);
    font-size: 11.5px;
    line-height: 1.3;
  }
  .count {
    margin-inline-start: auto;
    color: var(--faint);
    font-size: 12px;
  }
</style>
