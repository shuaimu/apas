# Dependency Vulnerability Remediation Verification

## Baseline (2026-08-08)

Toolchain used for the baseline:

- Node.js: 24.15.0
- npm: 11.12.1
- pnpm: 10.28.0 (invoked through `npx` because pnpm is not installed globally)

### npm

- Full graph: 15 vulnerable package entries (1 critical, 12 high, 1 moderate, 1 low).
- Production-only graph: 4 vulnerable package entries (4 high): `next`, `postcss`, `sharp`, and `nanoid`.
- Development/build graph adds: `vitest` (critical), `vite`, `rollup`, `ws`, `minimatch`, `picomatch`, `brace-expansion`, `js-yaml`, `flatted`, `ajv`, and `@babel/core`.
- Direct vulnerable dependencies: `next`, `postcss`, and `vitest`.

### pnpm

- Full graph: 76 advisory/path findings (1 critical, 43 high, 28 moderate, 4 low).
- Production-only graph: 39 advisory/path findings (18 high, 17 moderate, 4 low).
- pnpm reports one entry per affected advisory/path, while npm aggregates findings by vulnerable package; the counts are therefore not directly comparable.
- The affected package set agrees with the npm baseline.

The baseline audit output was captured without registry credentials or private configuration. Final resolved versions, audit results, overrides, and compatibility evidence are recorded below as implementation proceeds.

## Resolution Mapping

The vulnerable production paths are rooted in Next.js and PostCSS:

- `next@16.1.1` directly supplies vulnerable Next.js code, `postcss@8.4.31`, and optional `sharp@0.34.5`.
- Direct `postcss@8.5.6` and Next.js's nested PostCSS resolve vulnerable Nano ID versions.

The vulnerable development/build paths are rooted in the test, lint, and DOM toolchain:

- `vitest@4.0.17` resolves vulnerable Vite, Rollup, picomatch, and PostCSS/Nano ID versions.
- `jsdom@27.4.0` resolves vulnerable `ws@8.19.0`.
- ESLint and `eslint-config-next@16.1.1` resolve vulnerable AJV, minimatch/brace-expansion, flatted, js-yaml, picomatch, and Babel versions.
- `@vitejs/plugin-react@5.1.2` also resolves the vulnerable Babel and Vite branches.
- `typescript-eslint@8.53.0` resolves the vulnerable minimatch/brace-expansion and picomatch branches.

The same parent families appear in `pnpm-lock.yaml`; pnpm expands repeated advisories by affected path rather than aggregating them by package.

Patched direct floors verified against registry metadata and the current audit ranges:

- `next` and matching `eslint-config-next`: 16.3.0 (Node.js >=20.9.0; current Node 24.15.0 satisfies it).
- `postcss`: 8.5.26 (pulls patched Nano ID >=3.3.17).
- `vitest`: 4.1.10 (supports Node 20, 22, and >=24 and Vite 6/7/8).

Next.js 16.3.0 resolves PostCSS 8.5.23 and optional Sharp ^0.35.3, both above their vulnerable ranges. The patched direct versions' React, TypeScript, ESLint, Node, and Vite peer/engine constraints are compatible with the current manifest and toolchain.

## Final Resolved Versions and Overrides

Key reviewed npm resolutions (the pnpm graph resolves compatible versions and passes its independent audit):

- Next.js / `eslint-config-next`: 16.3.0
- PostCSS: 8.5.26 direct and 8.5.23 under Next.js
- Sharp: 0.35.3
- Vitest: 4.1.10
- `@vitejs/plugin-react`: 6.0.5
- Vite: 8.2.1
- Babel core: 7.29.7
- AJV: 6.15.0
- brace-expansion: 1.1.18 and 2.1.4
- flatted: 3.4.4
- js-yaml: 4.3.1
- minimatch: 3.1.5 and 9.0.9
- Nano ID: 3.3.18
- picomatch: 2.3.2 and 4.0.5
- ws: 8.21.3

One narrow override is required. pnpm auto-installs Vite 8.2.1's optional esbuild peer and initially selected esbuild 0.27.7, which is affected by GHSA-g7r4-m6w7-qqqr. The npm and pnpm override forms both scope Vite's esbuild peer to 0.28.1. npm currently omits this optional peer, but carries the equivalent constraint so a future npm resolution cannot introduce the vulnerable line. Remove the override when a stable Vite release requires a non-vulnerable esbuild floor and both clean audits remain at zero without it.

The esbuild 0.28.1 override passed the complete Vitest suite, standalone TypeScript checking, and the optimized Vite/Next.js production build. No peer or engine warning remains after declaring Vite 8.2.1 explicitly alongside `@vitejs/plugin-react` 6.0.5.

Lockfile inspection found no non-npm registry URLs, no newly selected prerelease versions, no remaining duplicate vulnerable versions, and no application-source changes caused by dependency resolution. The larger mechanical lockfile diff is explained by the Next.js/Sharp platform packages and the Vite 8 move from Rollup/esbuild-based internals to Rolldown/Lightning CSS packages.

npm 11.12.1 reports `@img/sharp-wasm32@0.35.3` and its `@emnapi/runtime` dependency as extraneous after `npm ci`. Regenerating a fresh lockfile from only the reviewed manifest reproduced the same two entries, confirming this is current Sharp 0.35.3/npm optional-platform metadata rather than stale APAS lock state. Both entries are optional, the installed Linux Sharp path builds successfully, and the fresh tree still audits at zero vulnerabilities.

## Clean Install and Audit Results

- `npm ci` installed 567 packages from `package-lock.json`; SHA-256 checks confirmed `package.json` and `package-lock.json` were unchanged.
- `npm run audit:npm` completed successfully with 0 vulnerabilities at the low-or-higher threshold.
- An isolated `pnpm@10.28.0 install --frozen-lockfile` installed 569 packages; SHA-256 checks confirmed `package.json` and `pnpm-lock.yaml` were unchanged.
- `pnpm run audit:pnpm` completed successfully with 0 vulnerabilities at the low-or-higher threshold.
- The package scripts and Makefile audit target were exercised; network auditing remains separate from `prebuild`.

## Compatibility and Smoke Verification

- ESLint: passed with 0 errors and the same 3 existing React hook warnings.
- Vitest full suite: 70 files and 507 tests passed.
- Standalone TypeScript: `tsc --noEmit` passed after type-only fixture corrections for the stricter Vitest/Next.js toolchain; no application behavior or assertions were weakened.
- Versioned optimized build: Next.js 16.3.0 compiled, type-checked, and generated all 10 static routes successfully with web version 26.08.27.
- Production-like local start: Next.js 16.3.0 became ready on `127.0.0.1:3101` in 455 ms.
- Local route smoke: `/`, `/login`, `/machines`, `/share`, `/admin`, `/register`, `/forgot-password`, and `/reset-password` each returned HTTP 200.
- Static assets: all 18 unique `/_next/static` assets referenced by the smoke-tested pages returned HTTP 200; the expected web version was embedded in `.next`.
- Authentication/admin/machine/share and WebSocket/store focused suite: 8 files and 88 tests passed.
- Terminal attachment/presentation focused suite: 6 files and 46 tests passed, covering terminal bus, reconciliation, view mode, pane theme, chat input, and tabbed terminal behavior.
- API integration: `http://apas.mpaxos.com/health` returned the healthy `apas-server` response. No production deployment or state-changing authentication/session action was performed during apply.
