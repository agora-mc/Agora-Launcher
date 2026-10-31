/**
 * How the Library shows the hash check of an installed file. The words come from agora-core's
 * `HashReport` (the `verified` flag and the `basis` text); this only decides which headline goes
 * with them, so a file is never called verified when the core did not verify it.
 */

import type { CatalogHash, HashOrigin } from './api';

export interface HashOutcome {
  tone: 'verified' | 'unverified';
  headline: string;
  /** The core's own explanation, shown as written. */
  basis: string;
  /** Set when a differing expectation was accepted on purpose (`install anyway`). */
  acceptedPast: string | null;
}

const ORIGIN_LABEL: Record<HashOrigin, string> = {
  curator_pin: 'the curator’s pinned hash for this file',
  previous_install: 'the hash this file had when it was installed before',
};

export function hashOutcome(hash: CatalogHash): HashOutcome {
  return {
    tone: hash.verified ? 'verified' : 'unverified',
    headline: hash.verified
      ? 'Verified: the file matches a hash on record.'
      : 'Not verified: no published hash checked this file.',
    basis: hash.basis,
    acceptedPast: hash.confirmedPast
      ? `Installed anyway: the file differed from ${ORIGIN_LABEL[hash.confirmedPast]}.`
      : null,
  };
}
