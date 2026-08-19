import { DocsSidebar } from '@/components/DocsSidebar';
import { getDocNavSections } from '@/lib/docs';

/** Every documentation page shares one grouped sidebar, so navigation never
 *  changes shape as a reader moves between guides and reference pages. */
export default async function DocsLayout({ children }: { children: React.ReactNode }) {
  const sections = await getDocNavSections();
  return (
    <div className="flex flex-col gap-10 lg:flex-row">
      <DocsSidebar sections={sections} />
      <div className="panel min-w-0 flex-1">{children}</div>
    </div>
  );
}
