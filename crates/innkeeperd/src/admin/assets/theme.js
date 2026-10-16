// Applies Options > Time of day before the first paint: 'light' (day) or 'dark' (dusk) overrides the
// system. The same key as the game page, so the choice carries over on a shared origin.
(function () {
  'use strict';
  try {
    var t = localStorage.getItem('inn.theme');
    if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
  } catch (e) { /* follow the system */ }
  document.documentElement.classList.add('js');
}());
