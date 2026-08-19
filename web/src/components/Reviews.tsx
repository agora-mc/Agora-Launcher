import { getReviews } from '@/lib/db';

interface ReviewsProps {
  itemId: string;
}

function StarRating({ rating }: { rating: number }) {
  const clamped = Math.max(0, Math.min(5, Math.round(rating)));
  return (
    <span className="inline-flex items-center gap-0.5 text-amber-500">
      {Array.from({ length: 5 }, (_, i) => (
        <span key={i} className="text-lg">{i < clamped ? '★' : '☆'}</span>
      ))}
    </span>
  );
}

export default async function Reviews({ itemId }: ReviewsProps) {
  const reviews = await getReviews(itemId);

  return (
    <div className="panel">
      <h2 className="rule-gold mb-4 text-lg font-semibold text-ink">Reviews</h2>
      {reviews.length === 0 ? (
        <p className="text-sm text-ink-muted/75">No reviews yet.</p>
      ) : (
        <div className="space-y-4">
          {reviews.map((review, i) => (
            <div key={i} className="panel-inset p-4">
              <div className="mb-1 flex items-center gap-2 text-sm">
                <span className="font-medium">{review.author}</span>
                <StarRating rating={review.rating} />
                {review.created_at && (
                  <span className="text-gray-500 dark:text-gray-400">{review.created_at}</span>
                )}
              </div>
              {review.body && (
                <p className="text-sm text-gray-700 dark:text-gray-300">{review.body}</p>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
