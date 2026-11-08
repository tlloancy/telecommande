// La barre s'ouvre et se ferme sur le sigle. Aucun délai.
export function setupBar(el: HTMLElement, onAction: (a: string) => void) {
  let open = false;
  const paint = () => el.classList.toggle("hidden", !open);
  el.addEventListener("click", e => {
    const btn = (e.target as Element).closest("button");
    const act = btn?.dataset.act;
    if (act) onAction(act);
  });
  return {
    toggle() { open = !open; paint(); return open; },
  };
}
