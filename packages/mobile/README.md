# APAS Code mobile

Expo SDK 56 / React Native application for the APAS coding-session companion.
It intentionally contains no general chat inbox and never executes project
code on the device.

Use a development build because SecureStore, SQLCipher SQLite, notifications,
and the bundled terminal surface require native modules. Copy `.env.example`
only for local overrides; production profiles accept HTTPS/WSS endpoints only.

The display name and bundle/application identifiers are provisional until the
production signing and store ownership decision is finalized.
