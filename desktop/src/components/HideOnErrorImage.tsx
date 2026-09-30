import { useState } from 'react';

/**
 * An image inside community-written About text. Remote images can fail (the
 * app's CSP only allows a few hosts, and links rot), so a failed one is hidden
 * instead of leaving a broken-image placeholder.
 */
export function HideOnErrorImage({ node: _node, ...props }: React.ImgHTMLAttributes<HTMLImageElement> & { node?: unknown }) {
  const [failed, setFailed] = useState(false);
  if (failed) return null;
  return (
    // eslint-disable-next-line jsx-a11y/alt-text
    <img
      {...props}
      loading="lazy"
      className="max-w-full h-auto rounded-lg"
      onError={() => setFailed(true)}
    />
  );
}
