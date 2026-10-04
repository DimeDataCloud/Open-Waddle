// The pixel-art pond revealed under the blasted screen: sky, hills, water,
// reeds and lily pads, drawn at quarter resolution and scaled up crisp.

function seeded(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

export function drawPond(canvas: HTMLCanvasElement, width: number, height: number, seed = 7): void {
  const P = 4;
  const w = Math.ceil(width / P);
  const h = Math.ceil(height / P);
  const low = document.createElement("canvas");
  low.width = w;
  low.height = h;
  const g = low.getContext("2d")!;
  const rand = seeded(seed);
  const horizon = Math.round(h * 0.62);

  // Sky in banded steps, like an old palette.
  const sky = ["#7ec8f2", "#8fd0f4", "#a2d9f6", "#b6e2f7", "#c9eaf8"];
  sky.forEach((c, i) => {
    g.fillStyle = c;
    g.fillRect(0, Math.floor((i * horizon) / sky.length), w, Math.ceil(horizon / sky.length) + 1);
  });
  // Sun.
  g.fillStyle = "#fff3a8";
  const sx = Math.round(w * 0.8);
  const sy = Math.round(h * 0.16);
  for (let y = -6; y <= 6; y++) for (let x = -6; x <= 6; x++) if (x * x + y * y <= 36) g.fillRect(sx + x, sy + y, 1, 1);
  // Clouds.
  g.fillStyle = "#ffffff";
  for (let i = 0; i < 7; i++) {
    const cx = Math.round(rand() * w);
    const cy = Math.round(rand() * horizon * 0.5) + 4;
    const len = 10 + Math.round(rand() * 18);
    g.fillRect(cx, cy, len, 3);
    g.fillRect(cx + 3, cy - 2, len - 7, 2);
    g.fillRect(cx + 6, cy - 3, Math.max(2, len - 14), 1);
  }
  // Two layers of rolling hills.
  for (const [colour, base, amp, freq] of [
    ["#7fb069", horizon - 10, 7, 0.035],
    ["#5e9a4f", horizon - 3, 5, 0.06],
  ] as const) {
    g.fillStyle = colour;
    const phase = rand() * 10;
    for (let x = 0; x < w; x++) {
      const top = Math.round(base - amp * (0.5 + 0.5 * Math.sin(x * freq + phase)));
      g.fillRect(x, top, 1, horizon - top + 1);
    }
  }
  // Water with darker depth bands and light ripples.
  const water = ["#3b8fc4", "#327fb1", "#2b6f9d", "#245f89"];
  water.forEach((c, i) => {
    g.fillStyle = c;
    const top = horizon + Math.floor(((h - horizon) * i) / water.length);
    g.fillRect(0, top, w, Math.ceil((h - horizon) / water.length) + 1);
  });
  g.fillStyle = "#8fd0f4";
  for (let i = 0; i < w / 3; i++) {
    const x = Math.round(rand() * w);
    const y = horizon + 2 + Math.round(rand() * (h - horizon - 4));
    g.fillRect(x, y, 2 + Math.round(rand() * 4), 1);
  }
  // Lily pads with the odd pink flower.
  for (let i = 0; i < w / 40; i++) {
    const x = Math.round(rand() * w);
    const y = horizon + 6 + Math.round(rand() * (h - horizon - 12));
    g.fillStyle = "#4caf50";
    g.fillRect(x, y, 6, 2);
    g.fillRect(x + 1, y - 1, 4, 1);
    g.fillStyle = "#2e7d32";
    g.fillRect(x + 3, y, 1, 1);
    if (rand() < 0.4) {
      g.fillStyle = "#f48fb1";
      g.fillRect(x + 1, y - 2, 2, 1);
    }
  }
  // Reeds along the shore.
  for (let i = 0; i < w / 6; i++) {
    const x = Math.round(rand() * w);
    const tall = 4 + Math.round(rand() * 8);
    g.fillStyle = "#3d7a34";
    g.fillRect(x, horizon - tall + 2, 1, tall);
    if (rand() < 0.35) {
      g.fillStyle = "#7b4a2a";
      g.fillRect(x, horizon - tall + 1, 1, 3);
    }
  }
  // A few fish shadows.
  g.fillStyle = "#1f557a";
  for (let i = 0; i < 6; i++) {
    const x = Math.round(rand() * w);
    const y = horizon + 10 + Math.round(rand() * (h - horizon - 14));
    g.fillRect(x, y, 4, 1);
    g.fillRect(x + 4, y - 1, 1, 3);
  }

  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d")!;
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(low, 0, 0, w * P, h * P);
}
