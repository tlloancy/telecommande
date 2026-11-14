import { describe, it, expect } from "vitest";
import { framePoint, hitScreen, place, zoomAt } from "./view";

describe("vue", () => {
  it("un écran choisi remplit le téléphone", () => {
    const box = place({
      vw: 360, vh: 240, frameW: 3360, frameH: 1080,
      region: { x: 1440, y: 0, w: 1920, h: 1080 },
      zoom: 1, panX: 0, panY: 0,
    });
    const shownLeft = box.left + 1440 * box.scale;
    expect(shownLeft).toBeCloseTo(0, 5);
    expect(1920 * box.scale).toBeCloseTo(360, 5);
  });

  it("le point sous les doigts ne bouge pas quand on zoome", () => {
    const args = {
      vw: 400, vh: 800, frameW: 1440, frameH: 960,
      region: { x: 0, y: 0, w: 1440, h: 960 },
      zoom: 1, panX: 0, panY: 0,
    };
    const before = place(args);
    const cx = 120, cy = 300;
    const fx = (cx - before.left) / before.scale;
    const next = zoomAt(args, cx, cy, 2);
    const after = place({ ...args, ...next });
    expect(after.left + fx * after.scale).toBeCloseTo(cx, 4);
    expect(next.zoom).toBe(2);
  });

  it("un toucher au milieu de l'écran choisi donne son centre", () => {
    const region = { x: 1440, y: 0, w: 1920, h: 1080 };
    const box = place({
      vw: 360, vh: 800, frameW: 3360, frameH: 1080,
      region, zoom: 1, panX: 0, panY: 0,
    });
    const p = framePoint(box, region, 180, 400);
    expect(p?.x).toBe(2400);
    expect(p?.y).toBe(540);
  });

  it("un toucher sur la moitié droite choisit l'écran externe", () => {
    const screens = [
      { name: "eDP-1", x: 0, y: 0, w: 1440, h: 960 },
      { name: "DP-2", x: 1440, y: 0, w: 1920, h: 1080 },
    ];
    const box = place({
      vw: 800, vh: 400, frameW: 3360, frameH: 1080,
      region: { x: 0, y: 0, w: 3360, h: 1080 },
      zoom: 1, panX: 0, panY: 0,
    });
    const right = box.left + (1440 + 100) * box.scale;
    const midY = box.top + 100 * box.scale;
    expect(hitScreen(screens, box, right, midY)).toBe("DP-2");
    expect(hitScreen(screens, box, box.left + 10 * box.scale, midY)).toBe("eDP-1");
  });
});
