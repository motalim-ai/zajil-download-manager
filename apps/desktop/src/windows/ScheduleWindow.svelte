<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../lib/components/Icon.svelte";
  import FileName from "../lib/components/FileName.svelte";
  import StatusChip from "../lib/components/StatusChip.svelte";
  import { api } from "../lib/api";
  import { scheduleApi } from "../lib/scheduleApi";
  import { startWindow } from "../lib/windowStart";
  import { locale } from "../lib/i18n.svelte";
  import { num, size } from "../lib/format";
  import type { QueuePlan } from "../lib/bindings/QueuePlan";
  import type { AfterChoice } from "../lib/bindings/AfterChoice";
  import type { Countdown } from "../lib/bindings/Countdown";
  import type { DownloadView } from "../lib/bindings/DownloadView";

  // The schedule window (M5.5): the queues on the side, the chosen queue's
  // schedule and downloads. The schedule and the number at once are saved
  // with «Save»; what happens after the queue is kept for this run only.
  let plans = $state<QueuePlan[]>([]);
  let downloads = $state<DownloadView[]>([]);
  let chosen = $state("main");
  let picked = $state<number | null>(null);
  let problem = $state<string | null>(null);
  let note = $state<string | null>(null);
  let countdown = $state<Countdown | null>(null);
  let adding = $state(false);
  let newName = $state("");
  let deleting = $state(false);

  // The form, from the chosen queue's plan.
  let timed = $state(false);
  let start = $state("");
  let stopOn = $state(false);
  let stop = $state("");
  let days = $state<string[]>([]);
  let maxActive = $state(3);

  const plan = $derived(plans.find((p) => p.name === chosen));
  const rows = $derived(downloads.filter((d) => d.queue === chosen));
  const others = $derived(plans.filter((p) => p.name !== chosen));
  // The week as each language counts it.
  const week = $derived(
    locale.lang === "ar" ? ["sat", "sun", "mon", "tue", "wed", "thu", "fri"] : ["mon", "tue", "wed", "thu", "fri", "sat", "sun"],
  );
  const dirty = $derived(
    plan !== undefined &&
      (timed !== (plan.start !== null || plan.days.length > 0) ||
        (timed && (start !== (plan.start ?? "") || [...days].sort().join() !== [...plan.days].sort().join())) ||
        stopOn !== (plan.stop !== null) ||
        (stopOn && stop !== (plan.stop ?? "")) ||
        maxActive !== plan.max_active),
  );

  const queueName = (name: string) => (name === "main" ? locale.t("queue-main") : name);

  function fill(p: QueuePlan | undefined) {
    if (!p) return;
    timed = p.start !== null || p.days.length > 0;
    start = p.start ?? "";
    stopOn = p.stop !== null;
    stop = p.stop ?? "";
    days = [...p.days];
    maxActive = p.max_active;
  }

  async function load(keepForm: boolean) {
    try {
      const [fresh, snapshot] = await Promise.all([scheduleApi.plans(), api.snapshot()]);
      plans = fresh;
      downloads = snapshot.downloads;
      if (!fresh.some((p) => p.name === chosen)) chosen = fresh[0]?.name ?? "main";
      if (!keepForm) fill(fresh.find((p) => p.name === chosen));
    } catch (e) {
      problem = String(e);
    }
  }

  onMount(() => {
    const unlisten: (() => void)[] = [];
    void (async () => {
      await startWindow();
      unlisten.push(await api.onChange(() => void load(dirty)));
      unlisten.push(await scheduleApi.onCountdown((c) => (countdown = c)));
      unlisten.push(await scheduleApi.onAfterFailed((e) => (problem = `${locale.t("schedule-after-failed")} ${e}`)));
      countdown = await scheduleApi.countdown().catch(() => null);
      await load(false);
    })();
    return () => unlisten.forEach((u) => u());
  });

  function choose(name: string) {
    chosen = name;
    picked = null;
    problem = null;
    note = null;
    deleting = false;
    fill(plans.find((p) => p.name === name));
  }

  async function run(what: () => Promise<void>, done?: string) {
    problem = null;
    note = null;
    try {
      await what();
      if (done) note = done;
    } catch (e) {
      problem = String(e);
    }
  }

  async function create() {
    const name = newName.trim();
    await run(async () => {
      await scheduleApi.newQueue(name);
      adding = false;
      newName = "";
      chosen = name;
      await load(false);
    });
  }

  async function remove() {
    await run(async () => {
      await scheduleApi.deleteQueue(chosen);
      deleting = false;
      chosen = "main";
      await load(false);
    });
  }

  async function save() {
    await run(async () => {
      await scheduleApi.saveQueue(chosen, {
        max_active: maxActive,
        start: timed && start ? start : null,
        stop: stopOn && stop ? stop : null,
        days: timed ? days : [],
      });
      await load(false);
    }, locale.t("schedule-saved"));
  }

  async function startNow() {
    await run(async () => {
      if (dirty) await save();
      await scheduleApi.startQueue(chosen);
      await load(true);
    }, plan?.closed_now ? locale.t("schedule-closed-now") : locale.t("schedule-started"));
  }

  async function setAfter(choice: AfterChoice) {
    await run(async () => {
      await scheduleApi.setAfter(chosen, choice);
      await load(true);
    });
  }

  function toggleDay(day: string, on: boolean) {
    days = on ? [...days, day] : days.filter((d) => d !== day);
  }

  function summary(p: QueuePlan): string {
    if (p.start === null && p.stop === null && p.days.length === 0) return locale.t("schedule-summary-none");
    return `${p.start ?? "00:00"}–${p.stop ?? "24:00"}`;
  }

  const index = $derived(picked === null ? -1 : rows.findIndex((r) => r.id === picked));

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !adding && !deleting) void api.closeWindow();
  }

  const clock = (s: number) => `${num(locale.lang, locale.digits, Math.floor(s / 60))}:${num(locale.lang, locale.digits, s % 60).padStart(2, num(locale.lang, locale.digits, 0))}`;
</script>

<svelte:window {onkeydown} />

<main class="schedule" aria-labelledby="schedule-title">
  <h1 id="schedule-title" class="visually-hidden">{locale.t("schedule-title")}</h1>

  <aside class="side" aria-label={locale.t("schedule-queues")}>
    <h2 class="side-title">{locale.t("schedule-queues")}</h2>
    <ul class="queues">
      {#each plans as p (p.name)}
        <li>
          <button class="queue" class:on={p.name === chosen} aria-current={p.name === chosen} onclick={() => choose(p.name)}>
            <Icon name="queue" />
            <span class="q-text">
              <bdi class="q-name">{queueName(p.name)}</bdi>
              <span class="q-when" dir="ltr">{summary(p)}</span>
            </span>
            <span class="q-count">{num(locale.lang, locale.digits, downloads.filter((d) => d.queue === p.name).length)}</span>
          </button>
        </li>
      {/each}
    </ul>
    {#if adding}
      <form class="new" onsubmit={(e) => (e.preventDefault(), void create())}>
        <label for="new-name">{locale.t("schedule-new-name")}</label>
        <!-- svelte-ignore a11y_autofocus -->
        <input id="new-name" bind:value={newName} autofocus onkeydown={(e) => e.key === "Escape" && (e.stopPropagation(), (adding = false))} />
        <p class="hint">{locale.t("schedule-new-hint")}</p>
        <div class="row">
          <button class="btn primary" type="submit" disabled={!newName.trim()}>{locale.t("schedule-create")}</button>
          <button class="btn" type="button" onclick={() => (adding = false)}>{locale.t("schedule-cancel")}</button>
        </div>
      </form>
    {:else if deleting}
      <div class="new" role="alertdialog" aria-labelledby="delete-q">
        <p id="delete-q"><bdi class="q-name">{queueName(chosen)}</bdi><br />{locale.t("schedule-delete-confirm")}</p>
        <div class="row">
          <button class="btn danger" onclick={() => void remove()}>{locale.t("schedule-delete")}</button>
          <button class="btn" onclick={() => (deleting = false)}>{locale.t("schedule-cancel")}</button>
        </div>
      </div>
    {:else}
      <div class="row side-actions">
        <button class="btn" onclick={() => ((adding = true), (problem = null))}><Icon name="plus" />{locale.t("schedule-new")}</button>
        <button class="btn" disabled={chosen === "main"} title={chosen === "main" ? locale.t("schedule-main-stays") : undefined} onclick={() => (deleting = true)}>
          <Icon name="trash" />{locale.t("schedule-delete")}
        </button>
      </div>
    {/if}
  </aside>

  <section class="plan" aria-labelledby="plan-title">
    <h2 id="plan-title" class="plan-title"><Icon name="calendar-clock" /><bdi>{queueName(chosen)}</bdi></h2>

    {#if countdown}
      <div class="countdown" role="alert">
        <Icon name="alert" />
        <span class="c-text">
          <bdi>{queueName(countdown.queue)}</bdi> —
          {locale.t(countdown.action === "sleep" ? "schedule-countdown-sleep" : "schedule-countdown-shutdown")}
          <strong class="c-time" dir="ltr">{clock(countdown.seconds)}</strong>
        </span>
        <button class="btn" onclick={() => void scheduleApi.cancelAfter()}>{locale.t("schedule-countdown-cancel")}</button>
      </div>
    {/if}

    <div class="grid">
      <span class="label">{locale.t("schedule-start")}</span>
      <div class="field inline">
        <label class="check"><input type="radio" name="timed" checked={!timed} onchange={() => (timed = false)} />{locale.t("schedule-start-any")}</label>
        <label class="check">
          <input type="radio" name="timed" checked={timed} onchange={() => (timed = true)} />{locale.t("schedule-start-at")}
          <input type="time" class="time" bind:value={start} disabled={!timed} aria-label={locale.t("schedule-start-at")} />
        </label>
      </div>

      <span class="label">{locale.t("schedule-days")}</span>
      <div class="field">
        <div class="days" role="group" aria-label={locale.t("schedule-days")}>
          {#each week as day (day)}
            <label class="day" class:off={!timed}>
              <input type="checkbox" checked={days.includes(day)} disabled={!timed} onchange={(e) => toggleDay(day, e.currentTarget.checked)} />
              {locale.t(`schedule-day-${day}`)}
            </label>
          {/each}
        </div>
        <p class="hint">{locale.t("schedule-days-hint")}</p>
      </div>

      <span class="label">{locale.t("schedule-stop")}</span>
      <div class="field">
        <label class="check">
          <input type="checkbox" bind:checked={stopOn} />{locale.t("schedule-stop-at")}
          <input type="time" class="time" bind:value={stop} disabled={!stopOn} aria-label={locale.t("schedule-stop-at")} />
        </label>
        <p class="hint">{locale.t("schedule-stop-hint")}</p>
      </div>

      <label class="label" for="max-active">{locale.t("schedule-max-active")}</label>
      <div class="field">
        <input id="max-active" class="number" type="number" min="1" max="32" bind:value={maxActive} />
      </div>

      <label class="label" for="after">{locale.t("schedule-after")}</label>
      <div class="field">
        <select id="after" value={plan?.after ?? "nothing"} onchange={(e) => void setAfter(e.currentTarget.value as AfterChoice)}>
          <option value="nothing">{locale.t("schedule-after-nothing")}</option>
          <option value="shutdown">{locale.t("schedule-after-shutdown")}</option>
          <option value="sleep">{locale.t("schedule-after-sleep")}</option>
        </select>
        <p class="hint">{locale.t("schedule-after-hint")}</p>
      </div>
    </div>

    <div class="files-head">
      <h3 id="files-title">{locale.t("schedule-files")}</h3>
      <div class="row">
        <button class="btn" disabled={index <= 0} onclick={() => picked !== null && void run(() => scheduleApi.moveInQueue(picked!, true))}>
          <Icon name="chevron-up" />{locale.t("schedule-up")}
        </button>
        <button class="btn" disabled={index < 0 || index >= rows.length - 1} onclick={() => picked !== null && void run(() => scheduleApi.moveInQueue(picked!, false))}>
          <Icon name="chevron-down" />{locale.t("schedule-down")}
        </button>
        <label class="visually-hidden" for="move-to">{locale.t("schedule-move")}</label>
        <select
          id="move-to"
          disabled={index < 0 || others.length === 0}
          value=""
          onchange={(e) => {
            const to = e.currentTarget.value;
            e.currentTarget.value = "";
            if (to && picked !== null) void run(() => api.moveToQueue([picked!], to));
          }}
        >
          <option value="" disabled>{locale.t("schedule-move")}</option>
          {#each others as o (o.name)}
            <option value={o.name}>{queueName(o.name)}</option>
          {/each}
        </select>
      </div>
    </div>
    <div class="table" role="grid" aria-labelledby="files-title" aria-rowcount={rows.length}>
      {#each rows as d, i (d.id)}
        <div
          class="tr"
          class:on={d.id === picked}
          role="row"
          aria-selected={d.id === picked}
          tabindex="0"
          onclick={() => (picked = d.id)}
          onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), (picked = d.id))}
        >
          <span class="td n" role="gridcell">{num(locale.lang, locale.digits, i + 1)}</span>
          <span class="td name" role="gridcell"><FileName name={d.name} /></span>
          <span class="td size" role="gridcell">{d.size === null ? "?" : size(locale.lang, locale.digits, d.size)}</span>
          <span class="td" role="gridcell"><StatusChip status={d.status} held={d.needs_ok} browser={d.waits_for_browser} /></span>
        </div>
      {:else}
        <p class="empty">{locale.t("schedule-empty")}</p>
      {/each}
    </div>

    {#if problem}<p class="problem" role="alert"><Icon name="alert" />{problem}</p>{/if}
    {#if note}<p class="note" role="status">{note}</p>{/if}
  </section>

  <footer class="actions">
    <button class="btn go" onclick={() => void startNow()}><Icon name="play" />{locale.t("schedule-start-now")}</button>
    <button class="btn primary" disabled={!dirty} onclick={() => void save()}><Icon name="check" />{locale.t("schedule-save")}</button>
    <button class="btn end" onclick={() => void api.closeWindow()}>{locale.t("schedule-close")}</button>
  </footer>
</main>

<style>
  .schedule {
    display: grid;
    grid-template-columns: 220px 1fr;
    grid-template-rows: 1fr auto;
    height: 100%;
    background: var(--surface);
    box-sizing: border-box;
  }
  .side {
    grid-row: 1 / 3;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 16px 12px;
    background: var(--side);
    border-inline-end: 1px solid var(--line);
    min-height: 0;
  }
  .side-title {
    font-size: 13px;
    color: var(--muted);
    margin: 0 4px;
    font-weight: 600;
  }
  .queues {
    list-style: none;
    margin: 0;
    padding: 0;
    flex: 1;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .queue {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: start;
    padding: 7px 8px;
    border-radius: 8px;
    cursor: pointer;
  }
  .queue.on {
    background: var(--sel);
  }
  .q-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .q-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .q-when {
    font-size: 12px;
    color: var(--muted);
    text-align: start;
    unicode-bidi: plaintext;
  }
  .q-count {
    font-size: 12px;
    color: var(--muted);
    border: 1px solid var(--line);
    border-radius: 99px;
    padding: 0 7px;
    background: var(--surface);
  }
  .new {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
  }
  .new p {
    margin: 0;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .side-actions .btn {
    flex: 1;
  }
  .plan {
    padding: 16px 22px 8px;
    overflow: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .plan-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 18px;
    margin: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 18px;
    align-items: start;
  }
  .label {
    font-size: 13px;
    color: var(--muted);
    padding-top: 6px;
    max-width: 112px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .field.inline {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 4px 22px;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
  }
  .days {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .day {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 3px 6px 3px 5px;
    font-size: 12.5px;
  }
  .day:has(input:checked) {
    background: var(--sel);
    border-color: var(--accent);
  }
  .day.off {
    opacity: 0.55;
  }
  .hint {
    font-size: 12.5px;
    color: var(--muted);
    margin: 0;
  }
  input:not([type="checkbox"], [type="radio"]),
  select {
    font: inherit;
    color: var(--ink);
    background: var(--sunken);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 8px;
    min-height: 32px;
    box-sizing: border-box;
  }
  .time {
    direction: ltr;
    width: 120px;
  }
  .number {
    width: 90px;
  }
  #after {
    max-width: 260px;
  }
  .countdown {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--amber-bg);
    color: var(--amber);
    font-weight: 600;
  }
  .c-text {
    flex: 1;
  }
  .files-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
    border-top: 1px solid var(--line);
    padding-top: 12px;
  }
  .files-head h3 {
    font-size: 14px;
    margin: 0;
  }
  .table {
    border: 1px solid var(--line);
    border-radius: 8px;
    min-height: 150px;
    flex: 1 0 auto;
    overflow: auto;
  }
  .tr {
    display: grid;
    grid-template-columns: 34px 1fr 90px 150px;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border-bottom: 1px solid var(--line);
    cursor: default;
  }
  .tr:last-child {
    border-bottom: 0;
  }
  .tr.on {
    background: var(--sel);
  }
  .td {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }
  .n,
  .size {
    color: var(--muted);
  }
  .empty {
    color: var(--muted);
    font-size: 13px;
    padding: 10px 12px;
    margin: 0;
  }
  .problem {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--red);
    margin: 0;
  }
  .note {
    color: var(--green);
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 8px;
    padding: 12px 22px 16px;
    border-top: 1px solid var(--line);
  }
  .actions .end {
    margin-inline-start: auto;
  }
  .btn.go {
    color: var(--green);
  }
  .btn.danger {
    color: var(--red);
  }
</style>
