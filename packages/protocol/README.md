# APAS protocol

This package contains framework-neutral client contracts and pure domain
helpers shared by APAS web and mobile clients. Rust types in `crates/shared`
are the source of truth.

Run `npm run generate` after changing a public Rust wire type. The command
exports JSON Schema, generates TypeScript with the pinned generator, and writes
only checked-in generated artifacts. CI runs `npm run check:generated` and
fails when regeneration changes the tree.

Application code should import runtime validators for untrusted network data
and generated types for compile-time safety. UI, storage, sockets, and React
hooks do not belong in this package.
