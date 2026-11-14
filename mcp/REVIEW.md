# Instructions de test pour le relecteur

Aucun compte, aucun réseau : tout se passe en local, sur un bureau X11 (réel ou virtuel).

## 1. Compiler et lancer les tests

    sudo apt install libxkbcommon-dev x11-xserver-utils xvfb xterm xdotool
    cd mcp && cargo test && cargo build --release

## 2. Essai manuel sous un écran virtuel

    Xvfb :99 -screen 0 1280x800x24 &
    export DISPLAY=:99 TELECOMMANDE_MCP_ASSUME_YES=1
    xterm -geometry 40x10+100+100 -bg '#ff0000' &
    printf '%s\n' \
      '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
      '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
      '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"screenshot","arguments":{"max_dim":640}}}' \
      | target/release/telecommande-mcp | head -c 600

Attendu : la liste de 8 outils, puis une image PNG (base64) de 640x400 où la fenêtre rouge apparaît.

## 3. Garde-fous

    target/release/telecommande-mcp stop     # arrêt d'urgence
    # toute action renvoie « Arrêt d'urgence actif », y compris list_screens
    target/release/telecommande-mcp resume
    unset TELECOMMANDE_MCP_ASSUME_YES        # l'accord sur le bureau est demandé (zenity/kdialog)
    cat ~/.local/state/telecommande/mcp-actions.log   # une ligne JSON par action, sans le texte tapé

## 4. Test automatique complet

    python3 tests/e2e.py target/release/telecommande-mcp

(37 vérifications : protocole, capture, clic, frappe, arrêt en pleine saisie, refus d'accord,
journal.) Nécessite Xvfb, xterm et xdotool ; Pillow est optionnel.

Le plugin n'a pas d'outil shell ni de lecture de fichiers : seulement écran, souris et clavier.
