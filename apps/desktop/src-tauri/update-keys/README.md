# Zajil's update keys (ADR-0011, section 3)

The two **public** keys Zajil accepts an update from, as `tauri signer
generate` writes them (`<key>.pub`, one line of base64). They are built
into Zajil (`src/update.rs`); never a private key here.

- `signing.pub`: the key the release workflow signs with. Its private key
  is a secret of the `release` environment on GitHub, protected by its
  password, reached by the owner and the release workflow only.
- `emergency.pub`: the offline key, kept by the owner away from the
  internet, used only if the signing key leaks (ADR-0011, section 4).

While a file is empty, Zajil offers no update to install: the notice and
the release page only, as before.
