// Negative fixture: an operation-shaped METHOD signature (onReview(): void)
// must be rejected just like a property callback — not just `onReview?:`.
export interface OperationShapedProps {
  onReview(): void;
}

export function FixtureCallbackMethod() {
  return null;
}
