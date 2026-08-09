// Negative fixture: explicit relative component extension resolving outside
// the interactive area must be rejected.
import SomeController from '../../../external/components/SomeNewController.tsx';

export function fixtureRelative() {
  return <SomeController />;
}
