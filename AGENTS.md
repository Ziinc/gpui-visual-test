# AGENTS.md

- Workspace: `crates/visual-test` (library `gpui-visual-test`) and `crates/cli`
  (`cargo visual-test`).
- Checks before pushing: `cargo fmt --check`,
  `cargo clippy --workspace --all-targets --features gpui-visual-test/capture,gpui-visual-test/judges -- -D warnings`,
  `cargo test --workspace --features gpui-visual-test/judges`, and
  `cargo test -p gpui-visual-test --features capture --test capture`
  (needs lavapipe on Linux; see README).
- Keep `gpui-pre` pinned to the snapshot gpui-kit and gpui-styleguidist use.
- To review visual checks from the CLI, follow `skills/gpui-visual-test/SKILL.md`.
- Docs site: `website/` (Docusaurus, a single page at `src/pages/index.mdx`). Keep it in sync with the README and public API; check with `npm run build`.
