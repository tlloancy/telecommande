// Recadrage local de la vidéo. L'image décodée reste le bureau entier ;
// on n'en montre qu'une région, agrandie autour des doigts.

export type Region = { x: number; y: number; w: number; h: number };
export type Place = { left: number; top: number; width: number; height: number; scale: number };

/** Point du bureau sous le doigt. Rien si le doigt est hors de la région affichée. */
export function framePoint(box: Place, region: Region, x: number, y: number): { x: number; y: number } | null {
  if (box.scale === 0) return null;
  const fx = (x - box.left) / box.scale;
  const fy = (y - box.top) / box.scale;
  if (fx < region.x || fy < region.y || fx >= region.x + region.w || fy >= region.y + region.h) return null;
  return { x: Math.round(fx), y: Math.round(fy) };
}

/** Écran sous le doigt, en coordonnées du téléphone. */
export function hitScreen(
  screens: { name: string; x: number; y: number; w: number; h: number }[],
  box: Place,
  x: number, y: number,
): string | null {
  if (box.scale === 0) return null;
  const fx = (x - box.left) / box.scale;
  const fy = (y - box.top) / box.scale;
  const hit = screens.find(s => fx >= s.x && fx < s.x + s.w && fy >= s.y && fy < s.y + s.h);
  return hit?.name ?? null;
}

const MIN_ZOOM = 1;
const MAX_ZOOM = 4;

export function place(args: {
  vw: number; vh: number;
  frameW: number; frameH: number;
  region: Region;
  zoom: number;
  panX: number; panY: number;
}): Place {
  const { vw, vh, frameW, frameH, region, zoom, panX, panY } = args;
  const fit = Math.min(vw / region.w, vh / region.h);
  const scale = fit * zoom;
  return {
    scale,
    width: frameW * scale,
    height: frameH * scale,
    left: (vw - region.w * scale) / 2 - region.x * scale + panX,
    top: (vh - region.h * scale) / 2 - region.y * scale + panY,
  };
}

type Frame = {
  vw: number; vh: number;
  frameW: number; frameH: number;
  region: Region;
  zoom: number;
  panX: number; panY: number;
};

/** Le point (cx, cy) de l'écran du téléphone reste sous les doigts. */
export function zoomAt(frame: Frame, cx: number, cy: number, factor: number): { zoom: number; panX: number; panY: number } {
  if (!(factor > 0) || frame.zoom === 0) return { zoom: frame.zoom, panX: frame.panX, panY: frame.panY };
  const before = place(frame);
  if (before.scale === 0) return { zoom: frame.zoom, panX: frame.panX, panY: frame.panY };
  const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, frame.zoom * factor));
  if (zoom === MIN_ZOOM) return { zoom, panX: 0, panY: 0 };
  const fx = (cx - before.left) / before.scale;
  const fy = (cy - before.top) / before.scale;
  const fit = before.scale / frame.zoom;
  const unpanned = place({ ...frame, zoom, panX: 0, panY: 0 });
  return {
    zoom,
    panX: cx - fx * fit * zoom - unpanned.left,
    panY: cy - fy * fit * zoom - unpanned.top,
  };
}
