// Negative fixture: domain/ may not import framework packages.
import { useState } from 'react';

export function fixtureExternal() {
  return useState;
}
