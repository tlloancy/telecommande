import { describe, it, expect } from "vitest";
import { Trackpad, type Gesture } from "./gestures";

describe("Trackpad", () => {
  it("un tap rapide = clic gauche", () => {
    const out: Gesture[] = []; const t = new Trackpad(e => out.push(e));
    t.down(1, 10, 10, 0); t.up(1, 100);
    expect(out).toEqual([{ t: "button", b: 0, down: true }, { t: "button", b: 0, down: false }]);
  });
  it("écarter les doigts zoome", () => {
    const out: Gesture[] = []; const t = new Trackpad(e => out.push(e));
    t.down(1, 0, 100, 0); t.down(2, 80, 100, 0); t.move(2, 200, 100);
    const pinch = out.find(e => e.t === "pinch");
    expect(pinch && pinch.t === "pinch" && pinch.scale).toBeCloseTo(2.5, 5);
  });
  it("deux doigts vers le haut font monter le contenu", () => {
    const out: Gesture[] = []; const t = new Trackpad(e => out.push(e));
    t.down(1, 10, 80, 0); t.down(2, 40, 80, 0); t.move(1, 10, 40);
    expect(out.find(e => e.t === "scroll")).toEqual({ t: "scroll", dx: 0, dy: 40 });
  });
  it("en vue d'ensemble un tap choisit, il ne clique pas", () => {
    const out: Gesture[] = [];
    const t = new Trackpad(e => out.push(e), () => false, () => true);
    t.down(1, 12, 40, 0); t.up(1, 100);
    expect(out).toEqual([{ t: "pick", x: 12, y: 40 }]);
  });
  it("au doigt, le curseur va au point touché puis le tap clique", () => {
    const out: Gesture[] = [];
    const t = new Trackpad(e => out.push(e), () => false, () => true, () => true);
    t.down(1, 12, 40, 0); t.up(1, 100);
    expect(out.map(e => e.t)).toEqual(["aim", "button", "button"]);
    expect(out[0]).toEqual({ t: "aim", x: 12, y: 40 });
  });
  it("tap à 2 doigts = clic droit", () => {
    const out: Gesture[] = []; const t = new Trackpad(e => out.push(e));
    t.down(1, 10, 10, 0); t.down(2, 50, 10, 0); t.up(1, 100); t.up(2, 110);
    expect(out[0]).toEqual({ t: "button", b: 2, down: true });
  });
});
