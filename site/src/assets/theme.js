// Runs in <head> before the page paints, so a saved light or dark theme
// never flashes. A file rather than an inline script: the site's
// Content-Security-Policy allows scripts from the site only.
(function () {
  document.documentElement.className += ' js';
  try {
    var t = localStorage.getItem('zajil-theme');
    if (t === 'light' || t === 'dark') document.documentElement.setAttribute('data-theme', t);
  } catch (e) {}
})();
