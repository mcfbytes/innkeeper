// The back office's few behaviours: the time-of-day switch, menus that close politely, copy buttons
// and the confirmation box. Every page works without it; forms that change things carry a visible
// "I mean it" box that this script replaces with a dialog. No third-party code.
(function () {
  'use strict';
  var doc = document;

  function setTheme(mode) {
    try {
      if (mode === 'system') localStorage.removeItem('inn.theme');
      else localStorage.setItem('inn.theme', mode);
    } catch (e) { /* kept for this page only */ }
    if (mode === 'light' || mode === 'dark') doc.documentElement.dataset.theme = mode;
    else delete doc.documentElement.dataset.theme;
    if (window.AdminVignette) window.AdminVignette.paint();
  }
  doc.querySelectorAll('[data-theme-set]').forEach(function (b) {
    b.addEventListener('click', function () { setTheme(b.dataset.themeSet); });
  });

  // One menu open at a time; Escape and a click elsewhere close it.
  var menus = Array.prototype.slice.call(doc.querySelectorAll('.menubar details'));
  menus.forEach(function (m) {
    m.addEventListener('toggle', function () {
      if (m.open) menus.forEach(function (o) { if (o !== m) o.open = false; });
    });
  });
  doc.addEventListener('click', function (e) {
    menus.forEach(function (m) { if (!m.contains(e.target)) m.open = false; });
  });
  doc.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape') return;
    menus.forEach(function (m) {
      if (m.open) { m.open = false; var s = m.querySelector('summary'); if (s) s.focus(); }
    });
  });

  // Copy buttons: data-copy names the element whose text is copied.
  doc.querySelectorAll('[data-copy]').forEach(function (b) {
    b.addEventListener('click', function () {
      var src = doc.getElementById(b.dataset.copy);
      if (!src) return;
      var text = src.textContent.trim();
      var done = function () { var was = b.textContent; b.textContent = 'Copied'; setTimeout(function () { b.textContent = was; }, 1600); };
      if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).then(done, fallback);
      else fallback();
      function fallback() {
        var r = doc.createRange(); r.selectNodeContents(src);
        var sel = window.getSelection(); sel.removeAllRanges(); sel.addRange(r);
        try { doc.execCommand('copy'); done(); } catch (e) { /* the text is selected for ctrl+c */ }
      }
    });
  });

  // A file input with data-file-into fills the named hidden field with the file as base64url
  // when its form is sent (the game-table import); the file itself is not a form field.
  doc.querySelectorAll('input[type=file][data-file-into]').forEach(function (picker) {
    var form = picker.form;
    var into = form && form.elements[picker.dataset.fileInto];
    if (!into) return;
    form.addEventListener('submit', function (e) {
      if (into.value || !picker.files || !picker.files[0]) return;
      e.preventDefault();
      picker.files[0].arrayBuffer().then(function (buf) {
        var bytes = new Uint8Array(buf), text = '';
        for (var i = 0; i < bytes.length; i += 0x8000) {
          text += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
        }
        into.value = btoa(text).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
        picker.disabled = true;
        form.submit();
      });
    });
  });

  // The confirmation box. Its tickbox stands in for the dialog without script, so it must not
  // block the submit with browser validation once the dialog does the asking.
  var dialog = doc.getElementById('confirm-dialog');
  if (dialog && typeof dialog.showModal === 'function') {
    doc.querySelectorAll('form[data-confirm] input[name=confirm]').forEach(function (b) { b.required = false; });
    var pending = null;
    var q = dialog.querySelector('.q'), detail = dialog.querySelector('.detail');
    var yes = dialog.querySelector('[data-yes]'), no = dialog.querySelector('[data-no]');
    doc.querySelectorAll('form[data-confirm]').forEach(function (form) {
      form.addEventListener('submit', function (e) {
        if (form.dataset.confirmed === '1') return;
        e.preventDefault();
        pending = form;
        q.textContent = form.dataset.confirm;
        detail.textContent = form.dataset.detail || '';
        yes.textContent = form.dataset.yes || 'Yes, do it';
        dialog.showModal();
        no.focus();
      });
    });
    yes.addEventListener('click', function () {
      var form = pending; dialog.close();
      if (!form) return;
      var box = form.querySelector('input[name=confirm]');
      if (box) box.checked = true;
      form.dataset.confirmed = '1';
      form.submit();
    });
    no.addEventListener('click', function () { pending = null; dialog.close(); });
    dialog.addEventListener('cancel', function () { pending = null; });
  }
}());
