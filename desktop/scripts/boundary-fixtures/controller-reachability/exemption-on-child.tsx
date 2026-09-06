// An exemption belonging to a nested child must not launder its parent. If this
// stops producing a violation, the directive span has leaked past the opening tag.
export function Nested({ onPick }: { onPick: () => void }) {
  return (
    <div className="outer" onClick={onPick}>
      {/* controller-exempt: this reason belongs to the child, not the wrapper */}
      <span>inner</span>
    </div>
  );
}
