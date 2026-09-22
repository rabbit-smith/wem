# Local hooks

The pre-commit layer: what is caught before a commit exists, and what is left to
the verdict ([`development.md`](development.md#verification-ladder)).

## Enable

```bash
git config core.hooksPath .githooks     # once per clone
```

`make hooks` runs that command and prints a confirmation. The setting lives in
`.git/config`, so it is **per clone** — cloning, a fresh worktree and CI do not
carry it. That is why the hook is tracked at `.githooks/pre-commit` rather than
living in `.git/hooks/`, which no clone carries.

## What it checks, and why

| Step | Check | Rule |
| --- | --- | --- |
| 1 | `scripts/check_staged_assets.py` | media and bank payloads (`.wem`, `.bnk`, `.pck`, `.wav`, `.ogg`, `.mp3`, `.flac`) under `tests/` only, the sanctioned asset tree in [`tests/AGENTS.md`](../../tests/AGENTS.md) ("Versioned assets"); nothing staged from `corpus/` or `local/`; no unexpected entry at the repository root |
| 2 | `scripts/check_provenance.py --staged` | the provenance patterns, over the staged content |
| 3 | `ruff check` on the staged `.py` files | the lint step of `make test`, over the files this commit touches |

Why at commit time: the repository was put through a full history rewrite to
remove reverse-engineering coordinates, a machine home directory, a vendor module
name and third-party content references. Each of those commits would have been
refused here in about a second; the rewrite took hours and ended in a force-push.
Two stray download artefacts were also once found in the root, unignored and one
`git add -A` from being published. Step 2 is another lane's file: it is called
when present and skipped with a notice when not, so the two land independently.

## Boundaries

Fast, offline, kernel-free — and **not** the verdict. What is deliberately not
here, and where each of those lives instead:

| Not here | Where |
| --- | --- |
| `mypy --no-site-packages src reference`, and `ruff check` over the whole tree | `make test` |
| `cargo fmt` / `clippy` / `doc`, `cargo test --workspace` | `make test` |
| the suites | `make test-fast`, `make wem-bytes`, `make 2ch-long` |
| wheel and wasm bytes | `python3 scripts/wheel_smoke.py`; `make wasm-build` then `node js/test-node.mjs` |
| profile code generation, which reads the untracked development tree | `scripts/generate_profile_code.py`, with the wheel check after it |

CI runs the ladder and is the verdict (`.github/workflows/test.yml`). A check
skipped here — no `ruff` on `PATH`, no `scripts/check_provenance.py` in the
checkout — is still enforced there, and the hook prints why it skipped rather
than looking successful. `git commit --no-verify` stays legitimate: a hook on a
developer machine is early feedback, never the only enforcement.
