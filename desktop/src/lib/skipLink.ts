/**
 * Where a "Skip to ..." control sends focus, and how the landing shows it.
 *
 * A landing point is normally silent (a page heading after a route change gets
 * no ring, see index.css). A skip control is different: the user asked to go
 * there, so the place they land must show where focus went. The mark is set
 * only while the target holds focus, and comes off when focus leaves.
 */
export function focusSkipTarget(target: HTMLElement, options?: FocusOptions): void {
  target.setAttribute('data-skip-landing', '');
  target.addEventListener('blur', () => target.removeAttribute('data-skip-landing'), { once: true });
  target.focus(options);
}
