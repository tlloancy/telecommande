# telecommande (nom provisoire)

Télécommande de bureau Linux depuis le téléphone : PWA + démon Rust, WebRTC.
Voir `PLAN.md` pour le cadrage complet et les phases.

## État

Phase 1 : le démon capture le bureau X11, l’encode en H.264 et l’envoie en WebRTC.
Le WebSocket ne porte plus que la signalisation (offre, réponse, ICE), protégée par le jeton.
Deux DataChannels sont ouverts : `move` (non fiable) et `ctrl` (fiable). L’injection
uinput arrive en phase 3.

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

Ensuite le démon affiche un QR. Sur le téléphone, même Wi-Fi : scanner, accepter le
certificat auto-signé, l’écran du bureau apparaît. Le curseur de la machine ne bouge
pas encore (phase 3). Si l’image est trop large, relancer avec `--max-width 1920`.

Résultat attendu : vidéo fluide du bureau, sans écran blanc. L’overlay « Connexion… »
disparaît à la première image.

## Limites v1

Écran verrouillé / écran de connexion (GDM) non géré. Session X11 uniquement
(Wayland : phase 9).

## Sécurité

Jeton 256 bits, un par appareil, stocké dans `~/.config/telecommande/` (600).
Écoute LAN uniquement par défaut. Aucun secret dans le dépôt.
Licence : à choisir.
