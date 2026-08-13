/**
 * NEGATIVE FIXTURE — must violate the ambience boundary.
 * `features/ambience/` may never import a page.
 */

import { Home } from '@/pages/Home';

export function HomePage(): JSX.Element {
  return <Home onNavigateTab={() => undefined} />;
}
