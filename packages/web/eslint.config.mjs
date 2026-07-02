// Flat ESLint config (ESLint 9 / Next 16, which removed `next lint`).
//
// Scope is intentionally NARROW: it enforces the React Rules of Hooks and
// nothing else. A hook placed after a conditional early return (the exact bug
// that threw React error #310 — "rendered more hooks than during the previous
// render" — and took the whole web app down) is now a lint ERROR, and
// `npm run build` runs this first via the `prebuild` script, so such a
// violation fails the build/deploy instead of shipping.
//
// Keeping it hooks-only means the build gate won't start failing on unrelated
// pre-existing lint noise; the full next/core-web-vitals ruleset can be
// layered on later if desired.
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default [
  { ignores: [".next/**", "node_modules/**", "next-env.d.ts"] },
  {
    files: ["src/**/*.{ts,tsx,js,jsx}"],
    plugins: { "react-hooks": reactHooks },
    languageOptions: {
      parser: tseslint.parser,
      parserOptions: {
        ecmaFeatures: { jsx: true },
        sourceType: "module",
      },
    },
    rules: {
      // Errors -> non-zero exit -> fails `npm run build`.
      "react-hooks/rules-of-hooks": "error",
      // Advisory only (warnings don't fail the build).
      "react-hooks/exhaustive-deps": "warn",
    },
  },
];
