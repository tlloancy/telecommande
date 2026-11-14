# Télécommande

Télécommande de bureau Linux depuis le téléphone : PWA + démon Rust, WebRTC.
Voir `PLAN.md` pour le cadrage complet et les phases.

## État

Phase 1 validée : le téléphone affiche le bureau. L’encodeur matériel est choisi
automatiquement (`vaapih264enc` sur la machine de référence).
Le doigt déplace le curseur, le tap clique, deux doigts font défiler.
Trois sigles restent affichés : l’écran interne, l’écran externe, les deux.
Un tap sur l’image d’un écran l’isole aussi. Le pincement agrandit la vue.
Le clavier du téléphone écrit dans la session. La flèche range les sigles.
`/dev/uinput` doit être accessible au groupe `telecommande` (le démon ne tourne pas en root).

`gstreamer1.0-nice` et les paquets `-dev` GStreamer sont requis pour compiler et pour
que `webrtcbin` démarre. Sans `nicesrc`, le démon s’arrête avec un message explicite.

## Dépendances (Linux Mint / Ubuntu)

```bash
sudo apt install build-essential pkg-config libssl-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
  gstreamer1.0-nice gstreamer1.0-vaapi gstreamer1.0-x \
  libxkbcommon-dev
```

`gstreamer1.0-nice` est indispensable (sinon webrtcbin échoue silencieusement).

## Phase 1 — compiler, lancer, tester

```bash
cd web && npm install && npm test && npm run build
cd ../host && cargo test
cargo run -- --self-test
cargo run -- --web-dir ../web/dist
```

`--self-test` doit afficher : `self-test : des images de l'écran sont arrivées par WebRTC`
et quitter avec le code 0. Il faut une session X11 (`DISPLAY` défini).

Avant le premier lancement qui doit bouger la souris :

```bash
sudo groupadd --system telecommande
sudo usermod -aG telecommande "$USER"
sudo install -m 644 packaging/70-telecommande-uinput.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --name-match=uinput
```

Le groupe n’est pris en compte que dans un nouveau shell (`sg telecommande` ou une nouvelle session).

Ensuite le démon affiche un QR. Sur le téléphone, même Wi-Fi : scanner, accepter le
certificat auto-signé, l’écran du bureau apparaît. Un doigt déplace le curseur.
Si l’image est trop large, relancer avec `--max-width 1920`.

Résultat attendu : vidéo fluide du bureau, sans écran blanc. L’overlay « Connexion… »
disparaît à la première image.

## Limites v1

Écran verrouillé / écran de connexion (GDM) non géré. Session X11 uniquement
(Wayland : phase 9).

## Serveur MCP (pour les agents)

Le dossier `mcp/` contient `telecommande-mcp`, un serveur MCP local (stdio) : un agent (Grok Build,
Cursor…) peut voir l’écran et utiliser la souris et le clavier de cette machine. Accord sur le
bureau à chaque session, arrêt d’urgence (`telecommande-mcp stop`) et journal des actions.
Voir `mcp/README.md`. Fichiers pour un catalogue de plugins : `.mcp.json`, `.grok-plugin/plugin.json`,
`assets/logo.png`.

## Sécurité

Appairage par code à usage unique (15 min par défaut, `--pair-ttl`), échangé contre un jeton 256 bits, un par appareil, stocké dans `~/.config/telecommande/` (600).
Écoute LAN uniquement par défaut. Aucun secret dans le dépôt.
Licence : Apache 2.0, © 2026 Thomas Lloancy. Voir `LICENSE` et `NOTICE`.
