# Documentation

Start with the [README](../README.md) for what the project is. Then:

| Document | Read it when |
| --- | --- |
| [`guides/usage.md`](guides/usage.md) | Installing the encoder, encoding a file, calling it from each language |
| [`guides/development.md`](guides/development.md) | Building, running the gates, determinism and repository conventions |
| [`reference/architecture.md`](reference/architecture.md) | Changing a layer boundary or the encoding flow |
| [`reference/domain-model.md`](reference/domain-model.md) | Naming anything: the vocabulary is normative |
| [`reference/profiles.md`](reference/profiles.md) | Touching profile data or its provenance |
| [`reference/public-interface.md`](reference/public-interface.md) | Touching package exports, the API, or the CLI |
| [`findings/2ch-byte-exactness.md`](findings/2ch-byte-exactness.md) | Needing the 2ch evidence, its root causes, or its trust boundary |
| [`methodology/byte-exact-diagnosis.md`](methodology/byte-exact-diagnosis.md) | Chasing any byte difference against an external build |
| [`roadmap.md`](roadmap.md) | Asking what is proven today and what is left |

## How this tree is organized

- **`reference/`** holds normative surfaces. They describe how the system *is*;
  a change here is a change to the contract.
- **`guides/`** holds task instructions. They describe what a person *does*.
- **`findings/`** holds evidence for a completed result: the observation behind
  each root cause, the accounting that closes, the retractions, and the boundary
  of what the corpus proves. A finding links to reference material instead of
  restating it.
- **`methodology/`** holds reusable method, written only after it has actually
  decided a case.

Binding agent-conduct rules do not live here; they are the layered `AGENTS.md`
set at the repository root and in each subtree.

## Conventions

- One claim, one link. Numbers live in the finding or in profile data, not in
  prose that can drift.
- Relative links only, so the tree works from a checkout, a wheel, and a ZIP
  import alike.
- Anything a reader could act on goes in `guides/`; anything a reader could
  verify goes in `findings/`; anything a reader must obey goes in `reference/`
  or `AGENTS.md`.