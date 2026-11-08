// Moteur de gestes (mode trackpad). Pur, testable : il émet des événements, ne touche pas au DOM.
import type { Ev } from "./rtc";

const TAP_MAX_MS = 200, TAP_MAX_MOVE = 8, LONG_PRESS_MS = 350;
const accel = (d: number) => d * (1 + Math.min(Math.abs(d) / 20, 2));

export class Trackpad {
  private pts = new Map<number, { x: number; y: number; x0: number; y0: number; t0: number }>();
  private maxFingers = 0;
  private moved = false;
  private dragging = false;
  private lp?: number;

  constructor(private emit: (e: Ev) => void) {}

  down(id: number, x: number, y: number, now = performance.now()) {
    this.pts.set(id, { x, y, x0: x, y0: y, t0: now });
    this.maxFingers = Math.max(this.maxFingers, this.pts.size);
    if (this.pts.size === 1) {
      this.moved = false;
      this.lp = setTimeout(() => {
        if (!this.moved && this.pts.size === 1) { this.dragging = true; this.emit({ t: "button", b: 0, down: true }); }
      }, LONG_PRESS_MS);
    }
  }

  move(id: number, x: number, y: number) {
    const p = this.pts.get(id); if (!p) return;
    const dx = x - p.x, dy = y - p.y; p.x = x; p.y = y;
    if (Math.hypot(x - p.x0, y - p.y0) > TAP_MAX_MOVE) { this.moved = true; clearTimeout(this.lp); }
    const n = this.pts.size;
    if (n === 1) this.emit({ t: "move", dx: Math.round(accel(dx)), dy: Math.round(accel(dy)) });
    else if (n === 2) this.emit({ t: "scroll", dx: Math.round(dx), dy: Math.round(dy) }); // scroll naturel
    // TODO n === 3 : Alt+Tab ; pincement : zoom local
  }

  up(id: number, now = performance.now()) {
    const p = this.pts.get(id);
    clearTimeout(this.lp);
    if (p && !this.moved && now - p.t0 <= TAP_MAX_MS && !this.dragging) {
      if (this.maxFingers === 1) this.click(0);
      else if (this.maxFingers === 2) this.click(2);
    }
    this.pts.delete(id);
    if (this.pts.size === 0) {
      if (this.dragging) this.emit({ t: "button", b: 0, down: false });
      this.dragging = false; this.maxFingers = 0;
    }
  }

  private click(b: number) { this.emit({ t: "button", b, down: true }); this.emit({ t: "button", b, down: false }); }
}
