import { describe, it, expect } from "vitest";
import { Trackpad } from "./gestures";
import type { Ev } from "./rtc";

describe("Trackpad", () => {
  it("un tap rapide = clic gauche", () => {
    const out: Ev[] = []; const t = new Trackpad(e => out.push(e));
    t.down(1, 10, 10, 0); t.up(1, 100);
    expect(out).toEqual([{ t: "button", b: 0, down: true }, { t: "button", b: 0, down: false }]);
  });
  it("tap à 2 doigts = clic droit", () => {
    const out: Ev[] = []; const t = new Trackpad(e => out.push(e));
    t.down(1, 10, 10, 0); t.down(2, 50, 10, 0); t.up(1, 100); t.up(2, 110);
    expect(out[0]).toEqual({ t: "button", b: 2, down: true });
  });
});
