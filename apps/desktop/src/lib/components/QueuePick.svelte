<script lang="ts">
  // A download's queue: one of the list, or a new one named in place, the
  // last choice of the list (owner's request, 2026-10-29: «make a queue
  // for it» while adding). The parent styles the select and the input.
  import { tick } from "svelte";
  import { locale } from "../i18n.svelte";
  import { scheduleApi } from "../scheduleApi";

  let {
    names,
    value = $bindable(),
    id,
    disabled = false,
  }: { names: string[]; value: string; id: string; disabled?: boolean } = $props();

  const NEW = "\u0000new";
  // Made here: in the list at once, before the parent's list comes again.
  let made = $state<string[]>([]);
  const all = $derived([...names, ...made.filter((m) => !names.includes(m))]);
  let naming = $state(false);
  let name = $state("");
  let said = $state<string | null>(null);
  let input: HTMLInputElement | undefined = $state();

  async function picked(event: Event) {
    const chosen = (event.currentTarget as HTMLSelectElement).value;
    if (chosen !== NEW) {
      value = chosen;
      return;
    }
    (event.currentTarget as HTMLSelectElement).value = value;
    naming = true;
    await tick();
    input?.focus();
  }
  function stop() {
    naming = false;
    name = "";
    said = null;
  }
  async function create() {
    const wanted = name.trim();
    if (!wanted) return stop();
    try {
      await scheduleApi.newQueue(wanted);
      made = [...made, wanted];
      stop();
      value = wanted;
    } catch (e) {
      const text = String(e);
      said = text.includes("invalid queue name") ? locale.t("schedule-new-hint") : text.includes("already a queue") ? locale.t("tree-queue-exists") : text;
    }
  }
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      void create();
    } else if (event.key === "Escape") {
      // Only the naming ends, not the window.
      event.preventDefault();
      event.stopPropagation();
      stop();
    }
  }
</script>

{#if naming}
  <input
    bind:this={input}
    {id}
    dir="auto"
    bind:value={name}
    placeholder={locale.t("schedule-new-name")}
    aria-label={locale.t("schedule-new-name")}
    aria-invalid={said !== null}
    title={said ?? undefined}
    {onkeydown}
    onblur={() => !name.trim() && stop()}
  />
  {#if said}<small class="said" role="alert">{said}</small>{/if}
{:else}
  <select {id} {value} onchange={(e) => void picked(e)} {disabled}>
    {#each all as q (q)}<option value={q}>{q === "main" ? locale.t("queue-main") : q}</option>{/each}
    <option value={NEW}>{locale.t("queue-new-option")}</option>
  </select>
{/if}

<style>
  .said {
    color: var(--red);
    font-size: 11.5px;
    flex-basis: 100%;
  }
</style>
