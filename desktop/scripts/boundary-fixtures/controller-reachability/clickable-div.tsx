// A clickable div with no way to focus it. The original sin this check exists for.
export function ClickableDiv({ onPick }: { onPick: () => void }) {
  return <div className="card" onClick={onPick}>Pick me</div>;
}
