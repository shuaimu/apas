## 1. Baseline and Resolution Mapping

- [x] 1.1 Capture fresh npm and pnpm audit baselines, including severity totals, vulnerable package versions, advisory identifiers, and whether each finding affects runtime or development tooling.
- [x] 1.2 Map every vulnerable transitive package to its direct parent in both lockfiles and record the current Node, npm, and declared pnpm versions and relevant engine/peer constraints.
- [x] 1.3 Verify the candidate patched floors for Next.js, matching `eslint-config-next`, PostCSS, and Vitest against current advisories and stable package metadata before editing the manifest.

## 2. Direct and Transitive Dependency Remediation

- [x] 2.1 Update `package.json` to stable patched Next.js 16.x, matching `eslint-config-next`, PostCSS 8.x, and Vitest 4.x floors without introducing unrelated direct dependencies or major product changes.
- [x] 2.2 Regenerate `package-lock.json` with the repository's npm toolchain and confirm it resolves patched runtime dependencies, including Next.js's PostCSS and Sharp subgraph.
- [x] 2.3 Regenerate `pnpm-lock.yaml` with pnpm 10.28.0 and confirm its direct dependency versions remain compatible with the npm graph.
- [x] 2.4 Re-audit both graphs and resolve any remaining Babel, AJV, brace-expansion, flatted, js-yaml, minimatch, Nano ID, picomatch, Rollup, Sharp, Vite, or ws findings through compatible parent upgrades.
- [x] 2.5 Add only the narrow npm and pnpm overrides still required for fixable transitives that cannot be upgraded through their parents, and document each override's dependency path, safety check, and removal condition.
- [x] 2.6 Inspect the completed lockfile diffs for unexpected packages, prereleases, registry changes, peer/engine warnings, duplicate vulnerable versions, and accidental manifest or application changes.

## 3. Reproducible Security Verification

- [x] 3.1 Add package scripts and a Makefile target for explicit all-severity npm and pnpm dependency audits without adding network-dependent auditing to `prebuild`.
- [x] 3.2 Change documented production installs from `npm install` to lockfile-frozen `npm ci`, and document the contributor `pnpm install --frozen-lockfile` verification path in `CLAUDE.md`, `claude.md`, and generated deployment guidance as applicable.
- [x] 3.3 Prove a clean `npm ci` succeeds without modifying `package.json` or `package-lock.json`, then run a fresh npm audit and record zero production or development vulnerabilities.
- [x] 3.4 Prove a clean `pnpm install --frozen-lockfile` succeeds without modifying `package.json` or `pnpm-lock.yaml`, then run a fresh pnpm audit and record zero production or development vulnerabilities.
- [x] 3.5 Add `verification.md` evidence listing the original 15-package baseline, final resolved versions, audit summaries, and the rationale for every override or an explicit statement that none was needed.

## 4. Web Compatibility and Build Verification

- [x] 4.1 Run the full web lint and test suites under the patched graph and resolve only dependency-induced regressions without weakening existing assertions or security gates.
- [x] 4.2 Run TypeScript checking and an optimized, versioned Next.js production build, confirming the updated Next.js, PostCSS, Sharp, Vitest, and Vite toolchain emits no errors.
- [x] 4.3 Add or update focused regression tests where needed for root/login/machines/share/admin routing, authentication, WebSocket/store handling, terminal snapshot rendering, and referenced static assets.
- [x] 4.4 Start the optimized build in a production-like local or staged environment and smoke-test public pages, API health, authentication, WebSocket attachment, and terminal presentation; append results to `verification.md`.

## 5. Release and Rollback Readiness

- [x] 5.1 Update the web deployment runbook to require a timestamped source/build backup, reviewed lockfile sync, production `npm ci`, versioned build, service restart, route/static-asset checks, and recent-log inspection.
- [x] 5.2 Document rollback to the previous source and lockfile/build, including that rollback restores the vulnerable graph temporarily and therefore requires a corrected patched redeployment.
- [x] 5.3 Run strict OpenSpec validation and review the final change diff to confirm that only dependency remediation, required compatibility adjustments, verification tooling, and release documentation are included.
