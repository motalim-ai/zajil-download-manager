/* Zajil website: small, dependency-free enhancements, for the Arabic
   (right to left) and English (left to right) pages. */
(function () {
  'use strict';

  var root = document.documentElement;
  var rtl = root.dir !== 'ltr';
  var T = rtl ? {
    navClose: 'إغلاق القائمة',
    navOpen: 'فتح القائمة',
    langBack: 'عد إلى العربية',
    langTry: 'جرب التبديل إلى الإنجليزية',
    ready: 'جاهز للتحميل',
    running: 'يحمل الآن',
    done: 'اكتمل التحميل',
    doneLive: 'اكتمل التحميل في المحاكاة.',
    cut: 'اقطع الاتصال',
    reconnect: 'أعد الاتصال',
    cutStatus: 'انقطع الاتصال، وما وصل محفوظ',
    cutLive: 'انقطع الاتصال. ما وصل من الملف محفوظ.',
    resumed: 'يكمل من حيث توقف',
    resumedLive: 'عاد الاتصال، والتحميل يكمل من حيث توقف.',
    restarted: 'بدأ التحميل من جديد.',
    pressStart: 'اضغط «ابدأ من جديد» لتشغيل المحاكاة',
    conns: function (n) {
      if (n === 0) return 'لا اتصالات تعمل';
      if (n === 1) return 'اتصال واحد يعمل';
      if (n === 2) return 'اتصالان يعملان';
      if (n <= 10) return n + ' اتصالات تعمل';
      return n + ' اتصالاً يعمل';
    }
  } : {
    navClose: 'Close the menu',
    navOpen: 'Open the menu',
    langBack: 'Back to English',
    langTry: 'Try switching to Arabic',
    ready: 'Ready to download',
    running: 'Downloading',
    done: 'Download finished',
    doneLive: 'The download finished in the simulation.',
    cut: 'Cut the connection',
    reconnect: 'Reconnect',
    cutStatus: 'Connection lost, and what arrived is kept',
    cutLive: 'Connection lost. What arrived of the file is kept.',
    resumed: 'Going on from where it stopped',
    resumedLive: 'The connection is back, and the download goes on from where it stopped.',
    restarted: 'The download started again.',
    pressStart: 'Press “Start again” to run the simulation',
    conns: function (n) {
      if (n === 0) return 'No connections working';
      if (n === 1) return '1 connection working';
      return n + ' connections working';
    }
  };
  var reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
  var darkQuery = window.matchMedia('(prefers-color-scheme: dark)');

  function $(sel, ctx) { return (ctx || document).querySelector(sel); }
  function $$(sel, ctx) { return Array.prototype.slice.call((ctx || document).querySelectorAll(sel)); }

  /* ---------- Theme ---------- */

  var themeListeners = [];

  function currentTheme() {
    var t = root.getAttribute('data-theme');
    if (t === 'light' || t === 'dark') return t;
    return darkQuery.matches ? 'dark' : 'light';
  }

  function syncThemeButton() {
    var btn = $('[data-theme-toggle]');
    if (!btn) return;
    btn.setAttribute('aria-checked', String(currentTheme() === 'dark'));
  }

  function themeChanged() {
    syncThemeButton();
    themeListeners.forEach(function (fn) { fn(); });
  }

  var themeBtn = $('[data-theme-toggle]');
  if (themeBtn) {
    themeBtn.addEventListener('click', function () {
      var next = currentTheme() === 'dark' ? 'light' : 'dark';
      root.setAttribute('data-theme', next);
      try { localStorage.setItem('zajil-theme', next); } catch (e) {}
      themeChanged();
    });
  }
  darkQuery.addEventListener('change', themeChanged);
  syncThemeButton();

  /* ---------- Mobile navigation ---------- */

  var navBtn = $('[data-nav-toggle]');
  var nav = $('#site-nav');
  function setNav(open) {
    navBtn.setAttribute('aria-expanded', String(open));
    navBtn.setAttribute('aria-label', open ? T.navClose : T.navOpen);
    nav.classList.toggle('is-open', open);
  }
  if (navBtn && nav) {
    navBtn.addEventListener('click', function () {
      setNav(navBtn.getAttribute('aria-expanded') !== 'true');
    });
    $$('a', nav).forEach(function (a) { a.addEventListener('click', function () { setNav(false); }); });
    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && navBtn.getAttribute('aria-expanded') === 'true') {
        setNav(false);
        navBtn.focus();
      }
    });
  }

  /* ---------- The «Zajil» menu: the home page's sections ---------- */

  var subBtn = $('[data-subnav]');
  if (subBtn) {
    var setSub = function (open) { subBtn.setAttribute('aria-expanded', String(open)); };
    subBtn.addEventListener('click', function () {
      setSub(subBtn.getAttribute('aria-expanded') !== 'true');
    });
    document.addEventListener('click', function (e) {
      if (!subBtn.parentElement.contains(e.target)) setSub(false);
    });
    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && subBtn.getAttribute('aria-expanded') === 'true') {
        setSub(false);
        subBtn.focus();
      }
    });
    $$('a', subBtn.parentElement).forEach(function (a) { a.addEventListener('click', function () { setSub(false); }); });
  }

  /* ---------- Language switch: the knob slides, then the other page opens ---------- */

  var langSwitch = $('[data-lang-switch]');
  if (langSwitch) {
    langSwitch.addEventListener('click', function (e) {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0 || reduceMotion.matches) return;
      e.preventDefault();
      langSwitch.classList.add('is-switching');
      setTimeout(function () { window.location.href = langSwitch.href; }, 320);
    });
    // Coming back with the browser's Back button shows the page as it was left.
    window.addEventListener('pageshow', function () { langSwitch.classList.remove('is-switching'); });
  }

  /* ---------- Lanes: one light pulse per "connection" ---------- */

  $$('[data-lanes]').forEach(function (el) {
    var count = el.classList.contains('lanes--quiet') ? 16 : 22;
    var frag = document.createDocumentFragment();
    for (var i = 0; i < count; i++) {
      var lane = document.createElement('span');
      lane.className = 'lane';
      var d = 4 + Math.random() * 6;
      lane.style.setProperty('--d', d.toFixed(2) + 's');
      lane.style.setProperty('--delay', (-Math.random() * d).toFixed(2) + 's');
      frag.appendChild(lane);
    }
    el.appendChild(frag);
  });

  // Pause decorative animation while it is off screen.
  if ('IntersectionObserver' in window) {
    var pauseIO = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) { en.target.classList.toggle('is-paused', !en.isIntersecting); });
    });
    $$('[data-lanes]').forEach(function (el) { pauseIO.observe(el.parentElement); });
  }

  /* ---------- Scroll: reading progress, diving pigeon, hero tilt ---------- */

  var header = $('.site-header');
  if (!header) return;
  var rail = $('.flight-rail');
  var heroShot = $('.window--hero');
  var ticking = false;

  function onScroll() {
    ticking = false;
    var max = document.documentElement.scrollHeight - window.innerHeight;
    var p = max > 0 ? Math.min(1, Math.max(0, window.scrollY / max)) : 0;
    header.style.setProperty('--progress', p.toFixed(4));
    if (rail && rail.getClientRects().length) {
      rail.style.setProperty('--progress', p.toFixed(4));
      rail.style.setProperty('--bird-y', (p * rail.clientHeight).toFixed(1) + 'px');
    }
    if (heroShot && !reduceMotion.matches) {
      var top = heroShot.getBoundingClientRect().top;
      var vh = window.innerHeight;
      // 10deg when the shot's top touches the bottom of the screen, flat by 35% from the top.
      var t = Math.min(1, Math.max(0, (top - vh * 0.35) / (vh * 0.65)));
      heroShot.style.setProperty('--tilt', (t * 10).toFixed(2) + 'deg');
    }
  }
  function requestScroll() {
    if (!ticking) { ticking = true; requestAnimationFrame(onScroll); }
  }
  window.addEventListener('scroll', requestScroll, { passive: true });
  window.addEventListener('resize', requestScroll);
  onScroll();

  /* ---------- Screenshot tabs (RTL arrow keys) ---------- */

  $$('[data-tabs]').forEach(function (box) {
    var tabs = $$('[role="tab"]', box);
    function select(tab, focus) {
      tabs.forEach(function (t) {
        var on = t === tab;
        t.setAttribute('aria-selected', String(on));
        t.tabIndex = on ? 0 : -1;
        document.getElementById(t.getAttribute('aria-controls')).hidden = !on;
      });
      if (focus) tab.focus();
      tab.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    }
    tabs.forEach(function (tab, i) {
      tab.addEventListener('click', function () { select(tab, false); });
      tab.addEventListener('keydown', function (e) {
        var n = tabs.length;
        var next = null;
        // Right to left, the left arrow moves forward; left to right, the right one.
        var fwd = rtl ? 'ArrowLeft' : 'ArrowRight';
        var back = rtl ? 'ArrowRight' : 'ArrowLeft';
        if (e.key === fwd) next = tabs[(i + 1) % n];
        else if (e.key === back) next = tabs[(i - 1 + n) % n];
        else if (e.key === 'Home') next = tabs[0];
        else if (e.key === 'End') next = tabs[n - 1];
        if (next) { e.preventDefault(); select(next, true); }
      });
    });
  });

  /* ---------- One-click English demo ---------- */

  var langBtn = $('[data-lang-demo]');
  var mini = $('[data-mini-ui]');
  if (langBtn && mini) {
    langBtn.addEventListener('click', function () {
      var pressed = langBtn.getAttribute('aria-pressed') !== 'true';
      var toEnglish = rtl ? pressed : !pressed;
      langBtn.setAttribute('aria-pressed', String(pressed));
      langBtn.textContent = pressed ? T.langBack : T.langTry;
      var swap = function () {
        mini.setAttribute('dir', toEnglish ? 'ltr' : 'rtl');
        mini.setAttribute('lang', toEnglish ? 'en' : 'ar');
        $$('[data-ar]', mini).forEach(function (el) {
          el.textContent = el.getAttribute(toEnglish ? 'data-en' : 'data-ar');
        });
      };
      if (reduceMotion.matches) { swap(); return; }
      mini.style.setProperty('--flip-x', toEnglish ? '14px' : '-14px');
      mini.classList.add('is-flipping');
      setTimeout(function () {
        swap();
        mini.style.setProperty('--flip-x', toEnglish ? '-14px' : '14px');
        requestAnimationFrame(function () {
          requestAnimationFrame(function () { mini.classList.remove('is-flipping'); });
        });
      }, 220);
    });
  }

  /* ---------- Highlight the visitor's system ---------- */

  (function () {
    var ua = navigator.userAgent || '';
    var platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    var os = null;
    if (/android|iphone|ipad|ipod/i.test(ua)) os = null;
    else if (/win/i.test(platform) || /windows/i.test(ua)) os = 'windows';
    else if (/mac/i.test(platform) || /mac os x/i.test(ua)) os = 'mac';
    else if (/linux/i.test(platform) || /linux/i.test(ua)) os = 'linux';
    if (!os) return;
    var card = $('.os[data-os="' + os + '"]');
    if (!card) return;
    card.classList.add('is-yours');
    $('.os__badge', card).hidden = false;
  })();

  /* ---------- Segmented download simulation ----------
     Mirrors Zajil's rule: when a connection finishes its part,
     it takes half of what is left of the slowest part. */

  var demo = $('[data-demo]');
  if (!demo) return;

  var canvas = $('[data-demo-canvas]', demo);
  var ctx = canvas.getContext('2d');
  var bar = $('[data-demo-bar]', demo);
  var statusEl = $('[data-demo-status]', demo);
  var pctEl = $('[data-demo-pct]', demo);
  var connsEl = $('[data-demo-conns]', demo);
  var liveEl = $('[data-demo-live]', demo);
  var cutBtn = $('[data-demo-cut]', demo);
  var cutLabel = $('[data-demo-cut-label]', demo);
  var restartBtn = $('[data-demo-restart]', demo);

  var PER_CONN = 1 / 26;   // share of the file one connection moves per second
  var NET_CAP = 1 / 5.5;   // the line itself caps the total
  var MIN_SPLIT = 0.006;

  var segs = [];
  var cut = false;
  var done = false;
  var started = false;
  var visible = false;
  var last = 0;
  var rafId = 0;
  var lastAria = -1;
  var resumeTimer = 0;
  var colors = {};

  function readColors() {
    var cs = getComputedStyle(demo);
    colors.done = getComputedStyle($('.sw--done', demo)).backgroundColor;
    colors.head = getComputedStyle($('.sw--head', demo)).backgroundColor;
    colors.left = getComputedStyle($('.sw--left', demo)).backgroundColor;
    colors.gap = cs.getPropertyValue('--surface').trim() || '#fff';
    colors.paused = cs.getPropertyValue('--muted').trim() || '#888';
  }

  var connText = T.conns;

  function selectedConns() {
    var r = $('input[name="conns"]:checked', demo);
    return r ? parseInt(r.value, 10) : 16;
  }

  function newConn() {
    return { bias: 0.55 + Math.random() * 0.9, f: 1 };
  }

  function reset() {
    var n = selectedConns();
    segs = [];
    for (var i = 0; i < n; i++) {
      segs.push({ start: i / n, end: (i + 1) / n, pos: i / n, conn: newConn() });
    }
    cut = false;
    done = false;
    lastAria = -1;
    clearTimeout(resumeTimer);
    cutBtn.disabled = false;
    cutLabel.textContent = T.cut;
    setStatus(T.ready, '');
    draw();
    update();
  }

  function setStatus(text, cls) {
    statusEl.textContent = text;
    statusEl.className = 'demo__status' + (cls ? ' ' + cls : '');
  }

  function activeCount() {
    var c = 0;
    for (var i = 0; i < segs.length; i++) if (segs[i].conn && segs[i].pos < segs[i].end) c++;
    return c;
  }

  function speedOf(seg, active) {
    return Math.min(PER_CONN, NET_CAP / Math.max(1, active)) * seg.conn.bias * seg.conn.f;
  }

  function step(dt) {
    var active = activeCount();
    var i, s;
    for (i = 0; i < segs.length; i++) {
      s = segs[i];
      if (!s.conn || s.pos >= s.end) continue;
      s.conn.f = Math.min(1.5, Math.max(0.45, s.conn.f + (Math.random() - 0.5) * 1.2 * dt));
      s.pos = Math.min(s.end, s.pos + speedOf(s, active) * dt);
    }
    // Free connections take half of the slowest part's remainder.
    for (i = 0; i < segs.length; i++) {
      s = segs[i];
      if (!s.conn || s.pos < s.end) continue;
      var conn = s.conn;
      s.conn = null;
      var slow = null;
      var worst = 0;
      for (var j = 0; j < segs.length; j++) {
        var o = segs[j];
        if (!o.conn || o.pos >= o.end) continue;
        var left = o.end - o.pos;
        if (left < MIN_SPLIT * 2) continue;
        var eta = left / Math.max(1e-6, speedOf(o, active));
        if (eta > worst) { worst = eta; slow = o; }
      }
      if (slow) {
        var mid = slow.pos + (slow.end - slow.pos) / 2;
        segs.push({ start: mid, end: slow.end, pos: mid, conn: conn });
        slow.end = mid;
      }
    }
  }

  function progress() {
    var got = 0;
    for (var i = 0; i < segs.length; i++) got += segs[i].pos - segs[i].start;
    return Math.min(1, got);
  }

  function resizeCanvas() {
    var dpr = Math.min(2, window.devicePixelRatio || 1);
    var w = bar.clientWidth;
    var h = bar.clientHeight;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw();
  }

  function draw() {
    var w = bar.clientWidth;
    var h = bar.clientHeight;
    if (!w) return;
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = colors.left;
    ctx.fillRect(0, 0, w, h);
    // Position 0 of the file is where its text starts: the right edge in Arabic.
    function x(p) { return rtl ? w - p * w : p * w; }
    for (var i = 0; i < segs.length; i++) {
      var s = segs[i];
      var x0 = x(s.pos);
      var x1 = x(s.start);
      var a = Math.min(x0, x1);
      var b = Math.max(x0, x1);
      if (b - a > 0.2) {
        ctx.fillStyle = colors.done;
        ctx.fillRect(a, 0, b - a, h);
      }
      if (s.conn && s.pos < s.end) {
        ctx.fillStyle = cut ? colors.paused : colors.head;
        ctx.fillRect(rtl ? x0 - 3 : x0, 0, 3, h);
      }
      // a thin seam where each part begins
      ctx.fillStyle = colors.gap;
      ctx.fillRect(x(s.start) - 1, 0, 1.5, h);
    }
  }

  function update() {
    var p = progress();
    var pct = Math.floor(p * 100 + 1e-9);
    if (done) pct = 100;
    pctEl.textContent = pct + '%';
    connsEl.textContent = connText(cut || done ? 0 : activeCount());
    if (pct !== lastAria) {
      lastAria = pct;
      bar.setAttribute('aria-valuenow', String(pct));
      bar.setAttribute('aria-valuetext', pct + '%');
    }
  }

  function frame(t) {
    rafId = 0;
    if (!visible) return;
    var dt = last ? Math.min(0.05, (t - last) / 1000) : 0;
    last = t;
    if (!cut && !done) {
      step(dt);
      if (progress() >= 0.99999) {
        done = true;
        cutBtn.disabled = true;
        setStatus(T.done, 'is-done');
        liveEl.textContent = T.doneLive;
      }
    }
    draw();
    update();
    if (!done) loop();
  }

  function loop() {
    if (!rafId) rafId = requestAnimationFrame(frame);
  }

  function start() {
    started = true;
    if (!cut && !done) setStatus(T.running, '');
    last = 0;
    loop();
  }

  cutBtn.addEventListener('click', function () {
    if (!started) { reset(); start(); }
    cut = !cut;
    clearTimeout(resumeTimer);
    if (cut) {
      cutLabel.textContent = T.reconnect;
      setStatus(T.cutStatus, 'is-cut');
      liveEl.textContent = T.cutLive;
    } else {
      cutLabel.textContent = T.cut;
      setStatus(T.resumed, '');
      liveEl.textContent = T.resumedLive;
      resumeTimer = setTimeout(function () { if (!cut && !done) setStatus(T.running, ''); }, 2200);
    }
    last = 0;
    draw();
    update();
    loop();
  });

  restartBtn.addEventListener('click', function () {
    reset();
    liveEl.textContent = T.restarted;
    start();
  });

  $$('input[name="conns"]', demo).forEach(function (r) {
    r.addEventListener('change', function () { reset(); start(); });
  });

  themeListeners.push(function () { readColors(); draw(); });

  readColors();
  reset();
  if (reduceMotion.matches) {
    setStatus(T.pressStart, '');
  }

  if ('ResizeObserver' in window) new ResizeObserver(resizeCanvas).observe(bar);
  else window.addEventListener('resize', resizeCanvas);
  resizeCanvas();

  if ('IntersectionObserver' in window) {
    new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        visible = en.isIntersecting;
        if (visible) {
          if (!started && !reduceMotion.matches) start();
          else if (started) { last = 0; loop(); }
        }
      });
    }, { threshold: 0.35 }).observe(demo);
  } else {
    visible = true;
    if (!reduceMotion.matches) start();
  }
})();

/* How many times Zajil was downloaded (site/downloads.mjs writes it once a
   day, from GitHub's own counts): shown in the hero once known, never a
   guess. Same site only; nothing is sent about the visitor. */
(function () {
  'use strict';
  var slot = document.querySelector('[data-downloads]');
  if (!slot || !window.fetch) return;
  var ar = document.documentElement.dir !== 'ltr';
  fetch('/downloads.json', { cache: 'no-cache', credentials: 'omit' })
    .then(function (answer) { return answer.ok ? answer.json() : null; })
    .then(function (counts) {
      var n = counts && counts.total;
      if (typeof n !== 'number' || !(n > 0)) return;
      var shown = n.toLocaleString(ar ? 'ar' : 'en');
      slot.querySelector('span').textContent = ar ? 'نُزِّل ' + shown + ' مرة' : 'Downloaded ' + shown + ' times';
      slot.hidden = false;
    })
    .catch(function () {});
})();
