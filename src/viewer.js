/* Pixel viewport and character grid are separate. Preserve the terminal's 1:2 cell ratio. */
(function (root) {
  'use strict';
  function layout(cols, rows, width, height, targetWidth, targetHeight) {
    if (![cols, rows, width, height].every(v => Number.isFinite(v) && v > 0)) return null;
    const boxWidth = Number.isFinite(targetWidth) && targetWidth > 0 ? Math.min(width, targetWidth) : width;
    const boxHeight = Number.isFinite(targetHeight) && targetHeight > 0 ? Math.min(height, targetHeight) : height;
    const cellWidth = Math.min(boxWidth / cols, boxHeight / (rows * 2));
    const contentWidth = cols * cellWidth;
    const contentHeight = rows * cellWidth * 2;
    return { cellWidth, cellHeight: cellWidth * 2, contentWidth, contentHeight,
      left: (width - contentWidth) / 2, top: (height - contentHeight) / 2,
      boxWidth, boxHeight };
  }
  if (typeof module !== 'undefined' && module.exports) module.exports = { layout };
  if (typeof document === 'undefined') return;

  const canvas = document.getElementById('screen');
  const status = document.getElementById('status');
  const ctx = canvas.getContext('2d', { alpha: false });
  const query = new URLSearchParams(location.search);
  const targetWidth = Number(query.get('width')) || 385;
  const targetHeight = Number(query.get('height')) || 350;
  const debug = query.get('debug') === '1';
  let latest = null;
  const state = root.__COLONQ_STATE__ = { seq: 0, paintCount: 0, grid: [0, 0], viewport: [0, 0],
    target: [targetWidth, targetHeight], connected: false, receivedFps: 0, paintedFps: 0, drawMs: 0 };
  let statsStart = performance.now(), receivedCount = 0, paintedCount = 0;
  function updateStats() {
    const now = performance.now(), seconds = (now - statsStart) / 1000;
    if (seconds < 0.8) return;
    state.receivedFps = receivedCount / seconds;
    state.paintedFps = paintedCount / seconds;
    receivedCount = paintedCount = 0; statsStart = now;
    state.frameAgeMs = latest && latest.producedAt ? Math.max(0, Date.now() - latest.producedAt) : null;
    if (debug) {
      status.style.display = 'block';
      status.textContent = `接收 ${state.receivedFps.toFixed(1)}fps / 绘制 ${state.paintedFps.toFixed(1)}fps
` +
        `绘制耗时 ${state.drawMs.toFixed(1)}ms / 帧龄 ${state.frameAgeMs === null ? '?' : state.frameAgeMs}ms`;
      status.style.whiteSpace = 'pre';
      status.style.background = 'rgba(0,0,0,.8)';
    }
  }
  setInterval(updateStats, 1000);
  function rgb(c) { return `rgb(${c[0]},${c[1]},${c[2]})`; }

  function draw(frame) {
    const started = performance.now();
    const width = window.innerWidth, height = window.innerHeight;
    const box = layout(frame.w, frame.h, width, height, targetWidth, targetHeight);
    if (!box) return;
    const dpr = Math.max(1, Math.min(2, window.devicePixelRatio || 1));
    const pixelWidth = Math.round(width * dpr), pixelHeight = Math.round(height * dpr);
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth; canvas.height = pixelHeight;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = rgb(frame.bg);
    ctx.fillRect(0, 0, width, height);
    ctx.font = `bold ${box.cellHeight * 0.9}px "JetBrains Mono", "Fira Code", "DejaVu Sans Mono", monospace`;
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'center';
    // A monospace font's advance isn't guaranteed to be exactly half its height.
    // Fit glyph ink horizontally to the fixed grid rather than altering cell positions.
    const glyphScale = Math.min(1, box.cellWidth / Math.max(0.01, ctx.measureText('M').width));
    ctx.save();
    ctx.translate(box.left, box.top);
    ctx.scale(glyphScale, 1);
    let previousColor = -1;
    for (const cell of frame.cells) {
      const index = cell[0], codepoint = cell[1], colorIndex = cell[2];
      if (index < 0 || index >= frame.w * frame.h || codepoint < 32 || codepoint > 0x10ffff) continue;
      const color = frame.palette[colorIndex];
      if (!color) continue;
      if (colorIndex !== previousColor) { ctx.fillStyle = rgb(color); previousColor = colorIndex; }
      const x = index % frame.w, y = Math.floor(index / frame.w);
      ctx.fillText(String.fromCodePoint(codepoint), (x + 0.5) * box.cellWidth / glyphScale,
                   (y + 0.5) * box.cellHeight, box.cellWidth / glyphScale);
    }
    ctx.restore();
    Object.assign(state, { seq: frame.seq, paintCount: state.paintCount + 1,
      grid: [frame.w, frame.h], viewport: [width, height], layout: box });
    paintedCount += 1;
    state.drawMs = performance.now() - started;
    state.lastFrameProducedAt = frame.producedAt;
    if (!debug) status.style.display = 'none';
  }
  window.addEventListener('resize', () => { if (latest) draw(latest); });
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(() => { if (latest) draw(latest); });
  const stream = new EventSource('/stream');
  stream.onopen = () => { state.connected = true; };
  stream.onmessage = event => {
    try {
      const frame = JSON.parse(event.data);
      if (!Array.isArray(frame.cells) || !Array.isArray(frame.palette) || !Array.isArray(frame.bg)) return;
      if (!(frame.w > 0 && frame.h > 0 && frame.w <= 2048 && frame.h <= 1024)) return;
      receivedCount += 1;
      state.lastReceivedAt = Date.now();
      latest = frame;
      // Draw on data arrival, not rAF; OBS/off-screen pages may throttle animation callbacks.
      draw(frame);
    } catch (error) { state.lastError = String(error); }
  };
  stream.onerror = () => {
    state.connected = false; // EventSource reconnects itself; don't stack polling intervals.
    if (!latest) status.textContent = '未连接到 renderer，正在重试…';
  };
})(typeof window !== 'undefined' ? window : globalThis);
