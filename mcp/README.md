# telecommande-mcp

Serveur MCP **local** (stdio) qui laisse un agent (Grok Build, Cursor, tout client MCP) voir et
piloter le bureau X11 de cette machine. Aucun port ouvert, aucun réseau : le client le lance comme
sous-processus. Il réutilise `keyboard.rs` et `screens.rs` du démon (disposition AZERTY comprise).

## Installation

    cd mcp && cargo build --release        # dépendances : libxkbcommon-dev, xrandr
    install -m755 target/release/telecommande-mcp ~/.local/bin/

Configuration côté client (format MCP habituel) :

    { "mcpServers": { "telecommande": { "command": "telecommande-mcp" } } }

## Outils

`list_screens`, `screenshot` (PNG, réduit à 1568 px par défaut), `mouse_move`, `click`, `drag`,
`scroll`, `type_text`, `key` (« Enter », « ctrl+c », « alt+F4 »...). Les coordonnées sont celles de
la **dernière capture** ; le serveur les reconvertit en pixels du bureau.

## Garde-fous

- **Arrêt d'urgence** : `telecommande-mcp stop` coupe toute action immédiatement, même en pleine
  session (un glisser-déposer en cours relâche le bouton). `resume` le lève. Lie `telecommande-mcp stop`
  à un raccourci clavier de ton bureau pour avoir un vrai bouton.
- **Accord sur le bureau** : première action de la session, puis après 600 s d'inactivité
  (`--idle-timeout`), une boîte zenity ou kdialog demande l'autorisation. Un refus bloque 60 s.
  Sans zenity/kdialog, l'agent est refusé, sauf `TELECOMMANDE_MCP_APPROVE_CMD` (commande, code 0 = oui)
  ou `TELECOMMANDE_MCP_ASSUME_YES=1` (machine dédiée).
- **Journal** : `~/.local/state/telecommande/mcp-actions.log`, une ligne JSON par action
  (permissions 600). Le texte tapé n'y figure jamais, seulement sa longueur.
- Pas de commande shell, pas de lecture de fichiers : seulement écran, souris et clavier.

## Limites

X11 uniquement (XTest). Pas encore de presse-papier ni de capture avec le curseur dessiné.
Un agent peut taper ce qu'il veut dans une fenêtre ouverte (terminal compris) : l'accord sur le
bureau est la vraie barrière, à ne pas contourner avec `ASSUME_YES` sur une machine personnelle.
