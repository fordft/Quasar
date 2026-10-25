import type { AccountV1 } from "$shared/ipc/bindings";
/** ChatGPT display state; deliberately contains no token or authorization URL. */
export function accountFixture(overrides: Partial<AccountV1> = {}): AccountV1 {
  return {
    version: 1,
    available: true,
    signed_in: false,
    sharing: false,
    pending: false,
    active: null,
    email: null,
    name: null,
    profiles: [],
    migration_pending: false,
    fault: null,
    ...overrides,
  };
}
