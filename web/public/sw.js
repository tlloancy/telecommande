// Réseau d'abord : une PWA en cache ne doit pas figer une vieille page pendant qu'on règle l'hôte.
const CACHE = "telecommande-v1";
self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE));
  self.skipWaiting();
});
self.addEventListener("activate", (e) => {
  e.waitUntil(self.clients.claim());
});
self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  if (url.pathname === "/ws") return;
  e.respondWith(fetch(e.request).catch(async () => (await caches.match(e.request)) || Response.error()));
});
