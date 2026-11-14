// Moteur de gestes (mode trackpad). Pur, testable : il émet des événements, ne touche pas au DOM.
// Deux doigts : scroll si les doigts restent écartés pareil, pincement si l'écart change,
// déplacement de la vue si l'image est déjà agrandie.
import type { Ev } from "./rtc";

const TAP_MAX_MS = 200, TAP_MAX_MOVE = 8, LONG_PRESS_MS = 350;
const accel = (d: number) => d * (1 + Math.min(Math.abs(d) / 20, 2));

export type Gesture =
  | Ev
  | { t: "pinch"; scale: number; cx: number; cy: number }
  | { t: "pan"; dx: number; dy: number }
  | { t: "pick"; x: number; y: number }
  | { t: "aim"; x: number; y: number };

export class Trackpad {
  private pts = new Map<number, { x: number; y: number; x0: number; y0: number; t0: number }>();
  private maxFingers = 0;
  private moved = false;
  private dragging = false;
  private pinching = false;
  private span = 0;
  private lp?: number;

  constructor(
    private emit: (e: Gesture) => void,
    private zoomed: () => boolean = () => false,
    private overview: () => boolean = () => false,
    private direct: () => boolean = () => false,
  ) {}

  down(id: number, x: number, y: number, now = performance.now()) {
    this.pts.set(id, { x, y, x0: x, y0: y, t0: now });
    this.maxFingers = Math.max(this.maxFingers, this.pts.size);
    if (this.pts.size === 1) {
      if (this.direct()) this.emit({ t: "aim", x, y });
      this.moved = false;
      this.lp = setTimeout(() => {
        if ((this.overview() && !this.direct()) || this.moved || this.pts.size !== 1) return;
        this.dragging = true;
        this.emit({ t: "button", b: 0, down: true });
      }, LONG_PRESS_MS);
    }
    if (this.pts.size >= 2) this.span = this.pairSpan();
  }

  move(id: number, x: number, y: number) {
    const p = this.pts.get(id); if (!p) return;
    const dx = x - p.x, dy = y - p.y; p.x = x; p.y = y;
    if (Math.hypot(x - p.x0, y - p.y0) > TAP_MAX_MOVE) { this.moved = true; clearTimeout(this.lp); }
    const n = this.pts.size;
    if (n === 1 && this.direct()) this.emit({ t: "aim", x, y });
    else if (n === 1 && !this.overview()) this.emit({ t: "move", dx: Math.round(accel(dx)), dy: Math.round(accel(dy)) });
    else if (n === 2) this.twoFingers(dx, dy);
    // TODO n === 3 : Alt+Tab
  }

  private twoFingers(dx: number, dy: number) {
    const span = this.pairSpan();
    const prev = this.span;
    const grew = prev > 0 ? Math.abs(span - prev) : 0;
    this.span = span;
    if (this.pinching || (grew > 8 && grew >= Math.hypot(dx, dy) * 0.75)) {
      this.pinching = true;
      if (prev > 0 && span > 0) {
        const pts = [...this.pts.values()];
        this.emit({
          t: "pinch",
          scale: span / prev,
          cx: (pts[0].x + pts[1].x) / 2,
          cy: (pts[0].y + pts[1].y) / 2,
        });
      }
      return;
    }
    if (this.zoomed()) this.emit({ t: "pan", dx, dy });
    else this.emit({ t: "scroll", dx: Math.round(dx), dy: Math.round(-dy) });
  }

  private pairSpan(): number {
    const pts = [...this.pts.values()];
    if (pts.length < 2) return 0;
    return Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y);
  }

  up(id: number, now = performance.now()) {
    const p = this.pts.get(id);
    clearTimeout(this.lp);
    if (p && !this.moved && now - p.t0 <= TAP_MAX_MS && !this.dragging) {
      if (this.direct() && this.maxFingers === 1) this.click(0);
      else if (this.overview() && this.maxFingers === 1) this.emit({ t: "pick", x: p.x0, y: p.y0 });
      else if (!this.overview() && this.maxFingers === 1) this.click(0);
      else if (!this.overview() && this.maxFingers === 2) this.click(2);
    }
    this.pts.delete(id);
    if (this.pts.size < 2) { this.span = 0; this.pinching = false; }
    if (this.pts.size === 0) {
      if (this.dragging) this.emit({ t: "button", b: 0, down: false });
      this.dragging = false; this.maxFingers = 0;
    }
  }

  private click(b: number) { this.emit({ t: "button", b, down: true }); this.emit({ t: "button", b, down: false }); }
}
