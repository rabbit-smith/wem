# Contributing

Six rules, in order. The first three are absolute: a contribution that breaks one
of them is rejected, and one that lands anyway compromises the repository
permanently, because a public history is not retractable. Process — build,
verification ladder, the shared checkout, commit discipline — is in
[`docs/guides/development.md`](docs/guides/development.md) and the layered
[`AGENTS.md`](AGENTS.md) set; this file states the rules those documents assume.

## 1. No Audiokinetic source code, SDK headers, or leaked material

Never submit Audiokinetic source code, SDK headers, decompiled output, or
material from a leaked source tree — in any form, including in comments, test
fixtures or commit messages. This is the one category of contribution that has
actually drawn takedowns against projects in this space, and a single such
contribution would compromise the whole repository: every later claim it makes
would rest on a tree that contains material it must not.

## 2. No game assets and no third-party media

Never submit game assets or any other third-party copyrighted media. A test
input must be synthesised in process or generated deterministically by a script
in this repository, and a fixture must never name or embed a real commercial
work. An issue report is held to the same rule: reproduce a problem with an
input you generated, never with the asset that failed.

## 3. No encryption, decryption, or circumvention

Never add encryption, decryption, key extraction, or any circumvention of a
technical protection measure. The decoder refuses anything it does not hold a
compiled profile for. That is a design boundary of the product, not an
unimplemented feature, and a patch that relaxes it is out of scope by
construction.

## 4. Provenance in comments

Describe observed behaviour in terms of "the paired build". Do not record
disassembler symbol names, absolute addresses, module or section names, tool
names, or internal report references — not in comments, test names, docstrings,
commit messages or documents. Say what a value is and how it was established,
not where it was seen. The cleanliness suite enforces the machine-checkable part
of this (`tests/parity/test_distribution.py`); the rest is the author's
responsibility.

## 5. Mechanics

- One logical change per commit, with a conventional commit scope: `feat(rust):`,
  `test:`, `fix(profiles):`, `docs:`, `ci:`, `chore:`.
- Stage explicit paths (`git add <file…>`); never `git add -A`. Push only when
  the maintainer asks.
- Verification: the suites named in
  [`docs/reference/standards.md`](docs/reference/standards.md#what-is-established)
  are what establish the claims this repository makes. Run the smallest target
  that covers the change first, then the affected suite; the full `make test` is
  for phase boundaries and for changes that touch shared state, configuration,
  lockfiles or generated assets. Read exit codes explicitly rather than a piped
  verdict; the ladder is in
  [`docs/guides/development.md`](docs/guides/development.md#verification-ladder).
- A new module under `src/wwise_wem/` or new packaged data carries registration
  duties of its own, in the same commit; they are listed in
  [`AGENTS.md`](AGENTS.md#new-file-registration-checklist).
- Before adding a file kind, check `.gitignore`. Generated material stays out of
  the tree unless it is a deliberate, tested asset.

## 6. What a contribution confirms

By contributing you confirm that you have the right to submit the work, and that
it contains none of the material prohibited above: no Audiokinetic source or SDK
material, no material from a leaked source tree, no third-party media, and no
circumvention of a technical protection measure.