# Sécurité

- Jeton d'appairage aléatoire de 256 bits, un par appareil, révocable.
- Aucune entrée (souris/clavier) acceptée sans canal authentifié.
- Écoute sur le LAN uniquement par défaut ; accès distant via Tailscale.
- Secrets dans `~/.config/telecommande/`, permissions 600.

Signaler une vulnérabilité : ouvrir une issue privée sur le dépôt.
