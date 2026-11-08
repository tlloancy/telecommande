// Barre flottante qui se masque seule.
export function setupBar(el: HTMLElement, onAction: (a: string) => void) {
  let timer: number | undefined;
  const show = () => { el.classList.remove("hidden"); clearTimeout(timer); timer = window.setTimeout(() => el.classList.add("hidden"), 3500); };
  el.addEventListener("click", e => { const a = (e.target as HTMLElement).dataset.act; if (a) { onAction(a); show(); } });
  return show;
}
