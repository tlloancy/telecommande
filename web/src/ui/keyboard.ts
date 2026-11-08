// Saisie clavier téléphone → hôte, par différence de contenu.
//
// Principe : un champ caché contient toujours une sentinelle (2 espaces) + ce que
// l’utilisateur tape. À chaque événement `input`, on compare le contenu du champ à ce que
// l’hôte a déjà reçu (`mirror`) : on envoie N retours arrière pour ce qui a disparu,
// puis le texte ajouté. On ne bloque jamais l’événement (pas de preventDefault sur les
// insertions) : l’état du clavier Android (Gboard) reste cohérent avec le champ, que le
// clavier compose (prédiction, autocorrection) ou non.
//
// Pourquoi ça corrige les symptômes :
//  - lettres mangées : plus de filtre anti-doublon par délai (il avalait « ll » de « belle »),
//    plus de preventDefault qui désynchronisait la composition de Gboard ;
//  - retour arrière une fois sur deux : le champ n’est jamais vide, donc Gboard émet toujours
//    un événement de suppression, et plus aucune suppression n’est muette à cause d’un délai.

export type Ev =
  | { t: "key"; code: number; down: boolean }
  | { t: "text"; s: string };

export const SENTINEL = "  ";
const BACKSPACE = 14, ENTER = 28;

// Touches de navigation (clavier matériel / claviers qui envoient de vrais keydown). Codes evdev.
const NAV: Record<string, number> = {
  Escape: 1, Tab: 15, Delete: 111, Home: 102, End: 107,
  ArrowUp: 103, ArrowLeft: 105, ArrowRight: 106, ArrowDown: 108,
};

/** Événements à envoyer pour passer de `mirror` (déjà envoyé) à `next` (contenu du champ). */
export function opsFor(mirror: string, next: string): Ev[] {
  if (mirror === next) return [];
  let p = 0;
  const n = Math.min(mirror.length, next.length);
  while (p < n && mirror.charCodeAt(p) === next.charCodeAt(p)) p++;
  // Ne pas couper une paire de substitution (emoji) en deux.
  if (p > 0 && mirror.charCodeAt(p - 1) >= 0xd800 && mirror.charCodeAt(p - 1) <= 0xdbff) p--;
  const out: Ev[] = [];
  const removed = Array.from(mirror.slice(p)).length;
  for (let i = 0; i < removed; i++) {
    out.push({ t: "key", code: BACKSPACE, down: true }, { t: "key", code: BACKSPACE, down: false });
  }
  const added = next.slice(p);
  if (added) out.push({ t: "text", s: added });
  return out;
}

export function attachKeyboard(input: HTMLInputElement, send: (ev: Ev) => void) {
  let mirror = SENTINEL;
  let composing = false;

  const tap = (code: number) => {
    send({ t: "key", code, down: true });
    send({ t: "key", code, down: false });
  };

  const toEnd = () => { const l = input.value.length; input.setSelectionRange(l, l); };

  /** Remet le champ à la sentinelle (sans rien envoyer : l’hôte n’est pas concerné). */
  const reset = () => {
    input.value = SENTINEL;
    mirror = SENTINEL;
    toEnd();
  };

  const sync = () => {
    const next = input.value;
    for (const ev of opsFor(mirror, next)) send(ev);
    mirror = next;
  };

  // On ne recycle le champ que hors composition, pour ne pas perturber Gboard en plein mot.
  const recycle = () => {
    if (composing) return;
    if (input.value.length < SENTINEL.length || input.value.length > 64) reset();
  };

  input.addEventListener("compositionstart", () => { composing = true; });
  input.addEventListener("compositionend", () => { composing = false; sync(); recycle(); });
  input.addEventListener("input", () => { sync(); recycle(); });
  input.addEventListener("focus", reset);

  // Garde le curseur en fin de champ (le diff suppose qu’on édite toujours à la fin).
  document.addEventListener("selectionchange", () => {
    if (document.activeElement === input && !composing && input.selectionStart !== input.value.length) toEnd();
  });

  // Vraies touches (Entrée, et claviers qui n’envoient pas d’événements de composition).
  // Gboard envoie keyCode 229 / key « Unidentified » pour les lettres et le retour arrière :
  // ces cas passent par `input`, pas ici. Chaque chemin est exclusif, donc pas de doublon.
  input.addEventListener("keydown", e => {
    if (e.key === "Enter") { tap(ENTER); e.preventDefault(); return; }
    if (e.key === "Backspace" && !e.isComposing) { tap(BACKSPACE); e.preventDefault(); return; }
    const code = NAV[e.key];
    if (code && !e.isComposing) { tap(code); e.preventDefault(); }
  });

  return { reset };
}
