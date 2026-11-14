import { Link, type Layout } from "./rtc";
import { Trackpad } from "./gestures";
import { attachKeyboard } from "./ui/keyboard";
import { framePoint, hitScreen, place, zoomAt, type Region } from "./view";

// Le QR porte un code court (#p=..., usage unique) échangé contre un jeton d'appareil,
// ou directement un jeton permanent (#t=..., `--pair`). Le jeton est mémorisé.
const legacy = location.hash.match(/t=([0-9a-f]+)/);
const pairCode = location.hash.match(/p=([0-9a-f]+)/);
if (legacy || pairCode) history.replaceState(null, "", location.pathname);
if (legacy) localStorage.setItem("token", legacy[1]);

const state = document.getElementById("state")!;
const video = document.getElementById("screen") as HTMLVideoElement;
const link = new Link();
link.onStatus = (text, live) => {
  state.textContent = text;
  state.classList.toggle("off", live);
};
link.onStream = (stream) => {
  video.srcObject = stream;
  if (stream) video.play().catch(() => {});
};
video.addEventListener("playing", () => state.classList.add("off"));

async function redeem(code: string): Promise<string | null> {
  try {
    const res = await fetch("/pair", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ code }),
    });
    if (res.ok) {
      const { token } = (await res.json()) as { token: string };
      localStorage.setItem("token", token);
      return token;
    }
    state.textContent =
      res.status === 410 ? "Ce QR a expiré : scanne le nouveau, affiché sur l’ordinateur."
      : res.status === 429 ? "Trop d’essais : réessaie dans une minute."
      : "QR refusé : scanne celui affiché sur l’ordinateur.";
  } catch {
    state.textContent = "Bureau injoignable : vérifie le Wi-Fi.";
  }
  return null;
}

void (async () => {
  const stored = localStorage.getItem("token");
  // Déjà appairé : un QR périmé ne doit pas casser la connexion existante.
  const token = pairCode ? ((await redeem(pairCode[1])) ?? stored) : stored;
  if (token) link.connect(token);
  else if (!pairCode) state.textContent = "Scanne le QR affiché sur l’ordinateur.";
})();

let layout: Layout | null = null;
let focusName = "all";
let zoom = 1, panX = 0, panY = 0;

function seeingAll() {
  return !layout || layout.screens.length < 2 || !layout.screens.some(s => s.name === focusName);
}

function region(): Region | null {
  if (!layout) return null;
  const screen = focusName && focusName !== "all" ? layout.screens.find(s => s.name === focusName) : undefined;
  return screen ?? { x: 0, y: 0, w: layout.w, h: layout.h };
}

function markSigils() {
  const idx = layout?.screens.findIndex(s => s.name === focusName) ?? -1;
  const active = idx >= 0 ? String(idx) : "all";
  document.querySelectorAll<HTMLButtonElement>("#screens button[data-screen]").forEach(b => {
    const on = b.dataset.screen === active;
    b.classList.toggle("on", on);
    b.setAttribute("aria-pressed", on ? "true" : "false");
  });
}

function applyView() {
  markSigils();
  const r = region();
  if (!layout || !r || r.w === 0 || r.h === 0) {
    video.classList.remove("placed");
    return;
  }
  const box = place({
    vw: window.innerWidth, vh: window.innerHeight,
    frameW: layout.w, frameH: layout.h, region: r, zoom, panX, panY,
  });
  video.classList.add("placed");
  video.style.left = `${box.left}px`;
  video.style.top = `${box.top}px`;
  video.style.width = `${box.width}px`;
  video.style.height = `${box.height}px`;
}

function onPinch(scale: number, cx: number, cy: number) {
  const r = region();
  if (!layout || !r) return;
  const frame = {
    vw: window.innerWidth, vh: window.innerHeight,
    frameW: layout.w, frameH: layout.h, region: r, zoom, panX, panY,
  };
  const next = zoomAt(frame, cx, cy, scale);
  zoom = next.zoom; panX = next.panX; panY = next.panY;
  applyView();
}

link.onLayout = (msg) => {
  layout = msg;
  if (focusName !== "all" && !msg.screens.some(s => s.name === focusName)) focusName = "all";
  applyView();
};

function selectScreen(which: string) {
  zoom = 1; panX = 0; panY = 0;
  if (which === "all") {
    focusName = "all";
    applyView();
    link.send({ t: "focus", name: "" });
    return;
  }
  const screen = layout?.screens[Number(which)];
  if (!screen) return;
  focusName = screen.name;
  applyView();
  link.send({ t: "focus", name: screen.name });
}

function focusAt(x: number, y: number) {
  if (!layout || seeingAll() === false) return;
  const box = place({
    vw: window.innerWidth, vh: window.innerHeight,
    frameW: layout.w, frameH: layout.h,
    region: { x: 0, y: 0, w: layout.w, h: layout.h },
    zoom, panX, panY,
  });
  const name = hitScreen(layout.screens, box, x, y);
  if (!name) return;
  focusName = name;
  zoom = 1; panX = 0; panY = 0;
  applyView();
  link.send({ t: "focus", name });
  navigator.vibrate?.(8);
}

let direct = false;
const pointBtn = document.getElementById("point")!;
const paintDirect = () => {
  pointBtn.classList.toggle("on", direct);
  pointBtn.setAttribute("aria-pressed", direct ? "true" : "false");
};
const aimAt = (x: number, y: number) => {
  const r = region();
  if (!layout || !r) return;
  const box = place({
    vw: window.innerWidth, vh: window.innerHeight,
    frameW: layout.w, frameH: layout.h, region: r, zoom, panX, panY,
  });
  const p = framePoint(box, r, x, y);
  if (!p) return;
  link.send({ t: "abs", x: p.x, y: p.y });
};
const pad = new Trackpad(e => {
  if (e.t === "pinch") { onPinch(e.scale, e.cx, e.cy); return; }
  if (e.t === "pan") { panX += e.dx; panY += e.dy; applyView(); return; }
  if (e.t === "pick") { focusAt(e.x, e.y); return; }
  if (e.t === "aim") { aimAt(e.x, e.y); return; }
  link.send(e);
  if (e.t === "button" && e.down) navigator.vibrate?.(8);
}, () => zoom > 1, () => seeingAll() && !!layout && layout.screens.length > 1, () => direct);
const screens = document.getElementById("screens")!;
const opts = document.getElementById("opts")!;
const fold = document.getElementById("fold")!;
const keys = document.getElementById("keys") as HTMLInputElement;
const keypad = document.getElementById("keypad")!;
let keysOpen = false;
let pointingKeypad = false;
const paintKeys = (open: boolean) => {
  keypad.classList.toggle("on", open);
  keypad.setAttribute("aria-pressed", open ? "true" : "false");
};
const closeKeys = () => {
  keysOpen = false;
  paintKeys(false);
  keys.blur();
};
const kb = attachKeyboard(keys, ev => link.send(ev));
const openKeys = () => {
  keysOpen = true;
  paintKeys(true);
  kb.reset();
  keys.focus({ preventScroll: true });
};
keypad.addEventListener("pointerdown", e => {
  e.preventDefault();
  e.stopPropagation();
  pointingKeypad = true;
});
keypad.addEventListener("pointercancel", () => { pointingKeypad = false; });
keypad.addEventListener("pointerup", e => {
  e.preventDefault();
  e.stopPropagation();
  if (keysOpen) closeKeys();
  else openKeys();
  pointingKeypad = false;
});
keypad.addEventListener("click", e => { e.preventDefault(); e.stopPropagation(); });
keys.addEventListener("blur", () => {
  if (pointingKeypad) return;
  keysOpen = false;
  paintKeys(false);
});
document.getElementById("screens")!.addEventListener("click", e => {
  const btn = (e.target as Element).closest("button");
  if (!btn) return;
  if (btn.dataset.act === "direct") {
    direct = !direct;
    paintDirect();
    return;
  }
  if (btn.dataset.act === "bar") {
    const folded = screens.classList.toggle("folded");
    opts.toggleAttribute("inert", folded);
    fold.setAttribute("aria-expanded", folded ? "false" : "true");
    return;
  }
  const which = btn.dataset.screen;
  if (which) selectScreen(which);
});
const liftChrome = () => {
  const vv = window.visualViewport;
  const covered = vv ? Math.max(0, window.innerHeight - vv.height - vv.offsetTop) : 0;
  screens.style.bottom = `calc(env(safe-area-inset-bottom) + 12px + ${covered}px)`;
};
window.visualViewport?.addEventListener("resize", liftChrome);
window.visualViewport?.addEventListener("scroll", liftChrome);
window.addEventListener("resize", () => { liftChrome(); applyView(); });

const onBar = (e: Event) => (e.target as HTMLElement).closest?.("#bar, #screens");
window.addEventListener("pointerdown", e => {
  if (onBar(e)) return;
  if (keysOpen) { closeKeys(); return; }
  pad.down(e.pointerId, e.clientX, e.clientY);
});
window.addEventListener("pointermove", e => { if (!onBar(e)) pad.move(e.pointerId, e.clientX, e.clientY); });
window.addEventListener("pointerup", e => pad.up(e.pointerId));
window.addEventListener("pointercancel", e => pad.up(e.pointerId));

// Empêche la mise en veille du téléphone.
const keepAwake = () => { navigator.wakeLock?.request("screen").catch(() => {}); };
keepAwake();
// Le verrou d'écran saute dès que la page est masquée ; on le reprend, et on relance la liaison
// si le téléphone a changé de pièce (donc de borne Wi-Fi) ou s'est endormi entre-temps.
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") return;
  keepAwake();
  link.resume();
});
window.addEventListener("online", () => link.resume());
window.addEventListener("pageshow", () => link.resume());
if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
