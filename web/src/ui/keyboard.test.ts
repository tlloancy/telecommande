import { describe, it, expect } from "vitest";
import { opsFor, SENTINEL } from "./keyboard";

const bs = (n: number) => Array.from({ length: n }, () => [
  { t: "key", code: 14, down: true }, { t: "key", code: 14, down: false }]).flat();

/** Simule le champ : applique les événements reçus par l’hôte à un écran. */
function host(evs: { t: string; s?: string; code?: number; down?: boolean }[], screen = "") {
  for (const e of evs) {
    if (e.t === "text") screen += e.s;
    else if (e.t === "key" && e.code === 14 && e.down) screen = Array.from(screen).slice(0, -1).join("");
  }
  return screen;
}

describe("opsFor", () => {
  it("première lettre d’un mot : envoyée", () => {
    expect(opsFor(SENTINEL, SENTINEL + "b")).toEqual([{ t: "text", s: "b" }]);
  });
  it("lettre doublée (belle) : aucune perte", () => {
    let m = SENTINEL, screen = "";
    for (const c of "belle") { const n = m + c; screen = host(opsFor(m, n), screen); m = n; }
    expect(screen).toBe("belle");
  });
  it("retour arrière dans le mot", () => {
    expect(opsFor(SENTINEL + "bon", SENTINEL + "bo")).toEqual(bs(1));
  });
  it("retour arrière sur la sentinelle : toujours envoyé", () => {
    expect(opsFor(SENTINEL, " ")).toEqual(bs(1));
  });
  it("autocorrection : teh -> the", () => {
    const evs = opsFor(SENTINEL + "teh ", SENTINEL + "the ");
    expect(host(evs, "teh ")).toBe("the ");
  });
  it("accents et emoji", () => {
    expect(opsFor(SENTINEL, SENTINEL + "é")).toEqual([{ t: "text", s: "é" }]);
    expect(opsFor(SENTINEL + "😀", SENTINEL)).toEqual(bs(1));
  });
  it("rien à faire si identique", () => {
    expect(opsFor(SENTINEL + "a", SENTINEL + "a")).toEqual([]);
  });
});
