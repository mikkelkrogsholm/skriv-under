# Skriv Under

Build the consent-based manually shared token-link document signing application in Rust.

- Exactly one administrator: only the owner logs in. Owner-issued MCP tokens act as that administrator. No signup or recipient accounts.
- Recipients access only their invited document through personal bearer tokens.
- Run Cargo from this repository root. Keep `.cargo/config.toml` in effect.
- Build output defaults to `../.rust-builds/document-signing`, outside the checkout.
  Respect CARGO_TARGET_DIR overrides and never put runtime data in the build directory.
- Use the pinned toolchain, one Cargo workspace, and the existing space-saving profiles.
- Never put PDFs, the database, credentials, or runtime data in the build directory.
- Keep `var/`, `.env` and bearer links out of Git and logs.
- Use SQLite transactions for document state changes. Preserve replay protection,
  token rotation, expiry, access control, CSRF protection and explicit consent.
- Opening an invitation must never sign or consume it. Scanner access is not proof of reading.
- Final PDFs append a receipt; do not claim cryptographic PDF sealing or verified identity.
- Run `cargo fmt --check` and `cargo test --locked` for application changes.
- No e-mail delivery or recipient e-mail intake. The administrator creates and shares links manually.
- Back up DATA_DIR/link-key with the database. Expose links only in authenticated administrator pages or MCP responses.
- MCP tokens have full UI administrator parity, are stored as hashes, expire and can be revoked.
- Share domain validation and transactions between HTML and MCP. MCP never bypasses recipient consent.
- Before cleanup, ensure no builds are active; use `cargo clean` from this directory.

<!-- OPENWIKI:START -->

## OpenWiki

This repository has a generated `openwiki/` evidence index. It is optional just-in-time context, not required startup reading.

- Do not enumerate, preload, or search wikis at task start. Use retrieval when the user asks for it, when unfamiliar architecture or dependency behavior materially affects the task, or when source inspection leaves an important uncertainty. Stop once the question is grounded.
- When those conditions apply and OpenWiki retrieval tools are available, use `openwiki_search` for just-in-time context and `openwiki_read` for the relevant complete sections. If search returns `workspace_required`, ask which listed workspace to use and retry with its ID.
- Use `openwiki_list_workspaces` or `openwiki_list_wikis` when workspace membership itself needs to be discovered.
- If the retrieval tools are unavailable, read `openwiki/quickstart.md` and follow its links to the relevant pages.
- Treat source code and tests as authoritative. A brief's unknowns and review items are verification gaps, not automatic requirements.
- Prefer the narrowest quiet validation that proves the changed behavior. Preserve complete failure output.

Use the repository's documented maintenance process to refresh generated pages after source changes. Check its configuration and completed runs before relying on automatic updates. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->
