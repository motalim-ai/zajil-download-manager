<script lang="ts">
  import { onMount } from "svelte";
  import Mark from "../lib/components/Mark.svelte";
  import { api } from "../lib/api";
  import { locale } from "../lib/i18n.svelte";
  import { startWindow } from "../lib/windowStart";
  import { firstLink } from "../lib/links";

  // The drop box (M5.5, an IDM touch): a link dropped here opens
  // «Download file info» (or «Add a link» while one is open for another
  // link); a right-click hides it; it is moved by dragging it.
  let over = $state(false);
  let missed = $state(false);

  onMount(() => {
    document.body.classList.add("drop");
    void startWindow();
  });

  async function drop(event: DragEvent) {
    event.preventDefault();
    over = false;
    const data = event.dataTransfer;
    const url = data ? firstLink(data.getData("text/uri-list"), data.getData("text/plain")) : null;
    if (!url) {
      missed = true;
      setTimeout(() => (missed = false), 900);
      return;
    }
    try {
      await api.openAddInfo(url, locale.t("info-title"));
    } catch {
      await api.openAddLink(url, false, locale.t("addlink-title")).catch(() => {});
    }
  }
</script>

<!-- "deep": the mark drags the window too. -->
<div
  class="box"
  class:over
  class:missed
  role="region"
  aria-label={locale.t("drop-box")}
  title={locale.t("drop-hint")}
  data-tauri-drag-region="deep"
  ondragover={(e) => {
    e.preventDefault();
    over = true;
  }}
  ondragleave={() => (over = false)}
  ondrop={(e) => void drop(e)}
  oncontextmenu={(e) => {
    e.preventDefault();
    void api.setDropBox(false);
  }}
>
  <Mark size={38} />
</div>

<style>
  /* Only in the drop box's own window (the page's CSS is one file for
     every window: a bare html or body rule would change them all). */
  :global(body.drop) {
    overflow: hidden;
  }
  .box {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: var(--surface);
    border: 2px solid var(--line);
    /* Square: the window itself is not transparent, its corners are. */
    cursor: grab;
    transition: border-color 0.15s, background 0.15s;
  }
  .over {
    border-color: var(--accent);
    background: var(--sel);
  }
  .missed {
    border-color: var(--red);
  }
</style>
