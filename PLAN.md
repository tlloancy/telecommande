# Cadrage

État : phase 1 validée sur le téléphone (vidéo fluide, encodeur VaapiLegacy).
Phase 2 déjà en place (sélection automatique de l’encodeur).
Phase 3 : souris relative, choix d’écran, frappe dans la session.
Trois sigles choisissent l’écran interne, l’externe, ou les deux. La flèche les range.
Le pincement agrandit la vue. Modificateurs, raccourcis et mode tactile ne sont pas là.
Nom : Télécommande (`telecommande` sur GitHub).
Licence : tous droits réservés, Tom. Voir `LICENSE`.

Machine lue sur cet hôte pour la phase 1 : Linux Mint 22.3, Cinnamon 6.6.9, session X11,
Intel HD Graphics 620, écran interne 1440×960 et écran DP-2 1920×1080 (bureau 3360×1080).
L’encodeur présent est `vaapih264enc`. `vah264enc` n’est pas dans ce GStreamer.

## 0. Contexte et objectif

Application minimaliste, équivalent très simplifié de Splashtop Desktop : le téléphone sert de télécommande pour un PC Linux.

Priorités, dans l’ordre :

1. Ergonomie
2. Esthétique
3. Latence faible
4. Compatibilité Linux (priorité absolue côté hôte)

Hors périmètre v1 : transfert de fichiers, audio, multi-utilisateurs, session GDM/écran verrouillé, hôte Windows/macOS.

Environnement de référence :

- Hôte : Linux Mint, bureau Cinnamon, session X11 (défaut de Cinnamon). Le Wayland de Cinnamon est expérimental : support en phase 9, optionnel.
- Client : téléphone (Android en priorité, iOS toléré), via navigateur.
- Contrainte : pas d’Android Studio, donc aucune app native.

## 1. Choix techniques

Client : PWA. Web app installable, plein écran, sans compilation ni store. TypeScript + Vite. Vanilla ou Svelte, pas de framework lourd. CSS soigné écrit à la main.

Transport : WebRTC. Vidéo en UDP, chiffrement DTLS-SRTP intégré.

- DataChannel 1, non fiable et non ordonné (`ordered: false`, `maxRetransmits: 0`) : mouvements de souris.
- DataChannel 2, fiable et ordonné : clics, touches, presse-papier, commandes.

Réseau local d’abord (zéro config). À distance : Tailscale (pas de port à ouvrir, pas de TURN).

Hôte : un démon Rust unique.

- Capture et encodage : GStreamer (`gstreamer-rs`, élément `webrtcbin`).
- Capture X11 : `ximagesrc use-damage=false show-pointer=false` (le curseur est rendu côté téléphone).
- Capture Wayland (phase 9) : `pipewiresrc` via le portail xdg-desktop-portal ScreenCast, avec `restore_token`.
- Encodage, détection automatique au démarrage : `vah264enc` / `vaapih264enc`, puis `nvh264enc`, puis `x264enc tune=zerolatency speed-preset=ultrafast`.
- Codec : H.264 Constrained Baseline, GOP court, pas de B-frames.
- Serveur HTTPS + WebSocket (axum) : sert la PWA et la signalisation SDP/ICE.
- Entrées : `/dev/uinput` via la crate `evdev`. Souris en mouvement relatif (mode trackpad).
- Règle udev + groupe dédié : le démon ne tourne jamais en root.

Distribution : paquet `.deb` + service systemd utilisateur (`systemctl --user`).

Dépendances système (Linux Mint / Ubuntu) :

```bash
sudo apt install build-essential pkg-config libssl-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
  gstreamer1.0-nice gstreamer1.0-vaapi gstreamer1.0-x \
  libxkbcommon-dev libavahi-compat-libdnssd-dev
```

`gstreamer1.0-nice` est indispensable : sans lui, `webrtcbin` ne trouve pas `nicesrc`/`nicesink`.

## 2. Pièges connus

- Disposition clavier (AZERTY) : le téléphone envoie des caractères, uinput attend des keycodes. Convertir avec xkbcommon et la disposition réelle de l’hôte. Repli : xclip puis Ctrl+V. Les touches spéciales partent directement en keycode.
- HTTPS obligatoire. Sur le LAN : certificat auto-signé au premier lancement, QR. À distance : `tailscale cert`.
- Latence : viser moins de 60 ms de bout en bout sur le LAN. GOP court, pas de B-frames, bitrate adaptatif plus tard (`getStats`).
- Wi-Fi : reconnexion automatique, ICE restart, sans recharger la page.
- Écran verrouillé / écran de connexion : non géré en v1.
- Capture X11 lente : `ximagesrc` peut saturer le CPU en 4K. Réduction de résolution à l’encodage (`videoscale`, `--max-width`) réglable.
- Curseur : non capturé, dessiné par la PWA à partir des coordonnées de l’hôte (phase ultérieure).

## 3. Ergonomie

Appairage : au lancement, QR dans le terminal et notification bureau. Jeton mémorisé. Pas de compte, pas de saisie d’IP. Découverte mDNS : `nomdelamachine.local`.

Trackpad (défaut) :

| Geste | Action |
| --- | --- |
| 1 doigt glisse | déplace le curseur (avec accélération) |
| tap | clic gauche |
| tap à 2 doigts | clic droit |
| 2 doigts glissent | scroll naturel |
| appui long puis glisse | glisser-déposer |
| 3 doigts glissent | changer de fenêtre (Alt+Tab) |
| pincement | zoom de la vue (local) |

Écran tactile direct : le toucher positionne le curseur à l’endroit touché.

Interface : la vidéo occupe tout l’écran. Barre flottante qui se masque seule. Rangée de touches au-dessus du clavier système. Raccourcis personnalisables. Retour haptique, thème sombre, indicateur de latence discret. Portrait et paysage, `env(safe-area-inset-*)`, wake lock. États soignés : connexion, reconnexion, erreur.

## 4. Structure du dépôt

```
.
├── PLAN.md
├── README.md
├── LICENSE
├── SECURITY.md
├── host/
├── web/
└── packaging/
```

## 5. Phases

| Phase | Contenu | Critère |
| --- | --- | --- |
| 1 | Pipeline GStreamer X11 vers WebRTC, page de test minimale | Vidéo fluide sur le téléphone, LAN |
| 2 | Détection et sélection automatique de l’encodeur | Matériel utilisé si dispo, repli x264 sinon |
| 3 | uinput : souris relative, clics, scroll | Contrôle du curseur au doigt |
| 4 | Moteur de gestes complet + mode écran tactile | Tous les gestes du tableau |
| 5 | Clavier, mapping AZERTY, presse-papier, raccourcis | Taper « é à ç ù œ » correctement |
| 6 | Appairage QR, jeton, HTTPS, mDNS | Première connexion en moins de 15 s |
| 7 | Polish UI, bitrate adaptatif, reconnexion, haptique | Pas de décrochage, latence stable |
| 8 | Tailscale, .deb, systemd, README | Installation en une commande |
| 9 | Wayland via portail + restore_token | Idem sous Cinnamon Wayland |

Une phase à la fois. La suivante attend la validation de la précédente.

## 6. Sécurité

- Jeton d’appairage aléatoire de 256 bits, révocable, un par appareil.
- Écoute LAN uniquement par défaut. Accès distant seulement via Tailscale.
- Aucune entrée acceptée sans canal authentifié.
- Bouton de coupure d’urgence côté hôte (zone de notification), phase ultérieure.
- Aucun secret dans le dépôt : tout vit dans `~/.config/telecommande/` en permissions 600.
- Expiration des sessions inactives et limitation des tentatives d’appairage : phase 6.

## 7. Consignes

Dépendances limitées à GStreamer, axum, evdev, xkbcommon, plus le strict nécessaire (QR, mDNS, TLS, types WebRTC). Commenter le pipeline et le mapping clavier. Tests unitaires pour le mapping clavier et le moteur de gestes. Mesurer la latence avant d’optimiser. Le nom est Télécommande. La licence réserve tous les droits à Tom. L’esthétique se demande.
