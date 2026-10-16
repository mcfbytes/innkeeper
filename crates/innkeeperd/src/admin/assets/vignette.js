// The header picture: a clerk's desk in the Town Hall back office, painted with a few rectangles and
// an ordered dither on a 192x54 grid, then scaled up with hard pixels. Original artwork.
(function () {
  'use strict';
  var canvas = document.getElementById('vignette');
  if (!canvas || !canvas.getContext) return;
  var W = 192, H = 54;
  canvas.width = W; canvas.height = H;
  var g = canvas.getContext('2d');
  var BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

  var DAY = {
    wall: ['#e9d9a6', '#d8c488'], sky: ['#8fc6ef', '#cfe9f7'], sun: '#ffe45c', hill: '#7a9a34', hill2: '#5f7d28',
    frame: '#6b4a2a', deskTop: '#a8743c', deskTop2: '#8f6030', deskFront: '#6f4724', deskFront2: '#5e3b1d',
    skin: '#e6b48a', hair: '#4a3020', visor: '#3f8f5a', vest: '#7c3b2a', shirt: '#f4efe0', paper: '#fffaf0',
    paper2: '#e2d8bd', ink: '#b3261e', tray: '#4d6a8f', tray2: '#3a5272', lamp: '#2f7a4f', glow: '#fff1b0', board: '#8a6a3d', pin: '#c4301c'
  };
  var DUSK = {
    wall: ['#2d2650', '#241f44'], sky: ['#3a2a6e', '#e07a5a'], sun: '#fff0c0', hill: '#2a3550', hill2: '#1e2840',
    frame: '#9a7a50', deskTop: '#7a5230', deskTop2: '#664324', deskFront: '#4a2f18', deskFront2: '#3b2412',
    skin: '#d19c78', hair: '#2a1a10', visor: '#2e7a4c', vest: '#6a2f2a', shirt: '#d8d2c4', paper: '#efe6cf',
    paper2: '#c9bd9c', ink: '#e0493e', tray: '#5c7ca8', tray2: '#445e84', lamp: '#2f9a62', glow: '#ffd36a', board: '#6a5030', pin: '#ff6a6e'
  };

  function dusk() {
    var t = document.documentElement.dataset.theme;
    if (t === 'dark') return true;
    if (t === 'light') return false;
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
  }
  function rect(c, x, y, w, h) { g.fillStyle = c; g.fillRect(x, y, w, h); }
  // A vertical blend between two colours, dithered so it stays on the pixel grid.
  function wash(x, y, w, h, a, b) {
    var ca = hex(a), cb = hex(b);
    for (var j = 0; j < h; j++) {
      var t = h > 1 ? j / (h - 1) : 0;
      for (var i = 0; i < w; i++) {
        var th = (BAYER[((y + j) & 3) * 4 + ((x + i) & 3)] + 0.5) / 16;
        g.fillStyle = t > th ? b : a; g.fillRect(x + i, y + j, 1, 1);
      }
    }
    return [ca, cb];
  }
  function hex(h) { var n = parseInt(h.slice(1), 16); return [n >> 16 & 255, n >> 8 & 255, n & 255]; }

  function paint() {
    var P = dusk() ? DUSK : DAY;
    g.clearRect(0, 0, W, H);
    wash(0, 0, W, 40, P.wall[0], P.wall[1]);
    // The window: sky, sun, hills.
    rect(P.frame, 8, 4, 42, 30);
    wash(10, 6, 38, 26, P.sky[0], P.sky[1]);
    rect(P.sun, 36, 10, 5, 5); rect(P.sun, 35, 11, 7, 3);
    rect(P.hill2, 10, 24, 38, 8); rect(P.hill, 10, 27, 38, 5);
    for (var x = 10; x < 48; x += 2) rect(P.hill, x, 24 + ((x * 7) % 3), 2, 2);
    rect(P.frame, 28, 6, 2, 26); rect(P.frame, 10, 18, 38, 2);
    rect(P.frame, 6, 33, 46, 2);
    // A notice board with pinned slips.
    rect(P.frame, 126, 5, 40, 24); rect(P.board, 128, 7, 36, 20);
    rect(P.paper, 131, 10, 10, 8); rect(P.paper2, 131, 15, 10, 1); rect(P.paper, 145, 9, 8, 10); rect(P.paper, 154, 13, 8, 9);
    rect(P.pin, 135, 10, 2, 2); rect(P.pin, 148, 9, 2, 2); rect(P.pin, 157, 13, 2, 2);
    rect(P.paper2, 133, 20, 6, 1); rect(P.paper2, 147, 14, 5, 1); rect(P.paper2, 156, 18, 4, 1);
    // The clerk: shoulders, shirt, head, eyeshade, one raised eyebrow.
    rect(P.vest, 66, 28, 28, 14); rect(P.shirt, 76, 28, 8, 8);
    rect(P.vest, 70, 25, 20, 6);
    rect(P.skin, 72, 12, 16, 15); rect(P.skin, 71, 15, 18, 9);
    rect(P.hair, 71, 11, 18, 3); rect(P.hair, 70, 14, 2, 5); rect(P.hair, 88, 14, 2, 5);
    rect(P.visor, 69, 9, 22, 4); rect(P.visor, 68, 12, 24, 1);
    rect(P.hair, 75, 16, 4, 1); rect(P.hair, 82, 14, 4, 1);
    rect(P.frame, 76, 18, 2, 2); rect(P.frame, 83, 18, 2, 2);
    rect(P.deskFront2, 78, 24, 4, 1);
    // The desk.
    rect(P.deskTop, 0, 38, W, 5); rect(P.deskTop2, 0, 42, W, 1);
    wash(0, 43, W, 11, P.deskFront, P.deskFront2);
    rect(P.deskFront2, 94, 43, 2, 11); rect(P.deskFront2, 0, 43, 3, 11); rect(P.deskFront2, W - 3, 43, 3, 11);
    // A stamped paper, the ink pad and the rubber stamp.
    rect(P.paper, 100, 35, 24, 4); rect(P.paper2, 100, 38, 24, 1);
    rect(P.ink, 105, 35, 8, 1); rect(P.ink, 105, 38, 8, 1); rect(P.ink, 105, 35, 1, 4); rect(P.ink, 112, 35, 1, 4);
    rect(P.tray2, 52, 36, 14, 3); rect(P.ink, 54, 35, 10, 1);
    rect(P.hair, 56, 26, 6, 3); rect(P.hair, 57, 25, 4, 1);
    rect(P.frame, 58, 29, 2, 6); rect(P.hair, 54, 34, 10, 2);
    // The in-tray, stacked, with papers leaning out.
    rect(P.tray2, 134, 28, 22, 10); rect(P.tray, 134, 26, 22, 3); rect(P.tray2, 136, 30, 18, 1);
    rect(P.paper, 137, 22, 14, 5); rect(P.paper2, 137, 25, 14, 1); rect(P.paper, 140, 19, 10, 4); rect(P.paper2, 142, 20, 6, 1);
    rect(P.tray, 134, 33, 22, 5); rect(P.tray2, 134, 37, 22, 1);
    // A banker's lamp.
    rect(P.deskTop2, 168, 37, 10, 2); rect(P.frame, 172, 28, 2, 9); rect(P.lamp, 166, 24, 14, 5); rect(P.lamp, 168, 22, 10, 2);
    rect(P.glow, 168, 29, 10, 1);
    if (dusk()) { g.globalAlpha = 0.18; rect(P.glow, 150, 29, 40, 9); g.globalAlpha = 1; }
  }
  window.AdminVignette = { paint: paint };
  paint();
  if (window.matchMedia) {
    var mq = window.matchMedia('(prefers-color-scheme: dark)');
    if (mq.addEventListener) mq.addEventListener('change', paint);
  }
}());
