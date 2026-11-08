import { Link } from "./rtc";
import { Trackpad } from "./gestures";
import { setupBar } from "./ui/bar";

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

if (!token) state.textContent = "Scanne le QR affiché sur l'ordinateur.";
else link.connect(token);

const pad = new Trackpad(e => { link.send(e); if (e.t === "button" && e.down) navigator.vibrate?.(8); });
const showBar = setupBar(document.getElementById("bar")!, a => { if (a === "quit") { localStorage.removeItem("token"); location.reload(); } });

window.addEventListener("pointerdown", e => { pad.down(e.pointerId, e.clientX, e.clientY); showBar(); });
window.addEventListener("pointermove", e => pad.move(e.pointerId, e.clientX, e.clientY));
window.addEventListener("pointerup", e => pad.up(e.pointerId));
window.addEventListener("pointercancel", e => pad.up(e.pointerId));

// Empêche la mise en veille du téléphone.
navigator.wakeLock?.request("screen").catch(() => {});
if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
