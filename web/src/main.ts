import { Link, type Layout } from "./rtc";
import { Trackpad } from "./gestures";
import { attachKeyboard } from "./ui/keyboard";
import { hitScreen, place, zoomAt, type Region } from "./view";

// Le jeton arrive via le QR (#t=...) puis est mémorisé.
const m = location.hash.match(/t=([0-9a-f]+)/);
if (m) { localStorage.setItem("token", m[1]); history.replaceState(null, "", location.pathname); }
const token = localStorage.getItem("token");

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

if (!token) state.textContent = "Scanne le QR affiché sur l’ordinateur.";
else link.connect(token);

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

const pad = new Trackpad(e => {
  if (e.t === "pinch") { onPinch(e.scale, e.cx, e.cy); return; }
  if (e.t === "pan") { panX += e.dx; panY += e.dy; applyView(); return; }
  if (e.t === "pick") { focusAt(e.x, e.y); return; }
  link.send(e);
  if (e.t === "button" && e.down) navigator.vibrate?.(8);
}, () => zoom > 1, () => seeingAll() && !!layout && layout.screens.length > 1);
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
navigator.wakeLock?.request("screen").catch(() => {});
if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
