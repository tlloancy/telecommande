// Même logique que public/sw.js, qui est le fichier réellement servi.
const CACHE = "telecommande-v1";
const worker = self as unknown as ServiceWorkerGlobalScope;

worker.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE));
  worker.skipWaiting();
});
worker.addEventListener("activate", (e) => {
  e.waitUntil(worker.clients.claim());
});
worker.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  if (url.pathname === "/ws") return;
  e.respondWith(fetch(e.request).catch(async () => (await caches.match(e.request)) ?? Response.error()));
});
