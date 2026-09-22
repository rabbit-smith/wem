# Documentation

Start with the [README](../README.md) for what the project is.

## Three owners of written truth

Each fact has exactly one home:

- **Product norms — what must be true of the system:**
  [`reference/standards.md`](reference/standards.md). It states each norm and
  names the test that establishes it. The other reference documents hold the
  design of one area each: [architecture](reference/architecture.md) (layers,
  dependency direction, encoding flow), [domain model](reference/domain-model.md)
  (the vocabulary), [profiles](reference/profiles.md) (profile data and its
  provenance), [public interface](reference/public-interface.md) (exports, the
  `encode` API, the CLI).
- **How work is done here:** [`guides/development.md`](guides/development.md) —
  the verification ladder, lanes and the shared checkout, commit discipline and
  the code-writing standards. [`guides/usage.md`](guides/usage.md) holds the task
  instructions: install, encode, call it from each language.
- **Evidence and method:** [`findings/`](findings/) holds the evidence record for
  a completed result, [`methodology/`](methodology/) the reusable method.

The `AGENTS.md` set at the repository root and in each subtree holds the same
working rules for an agent, plus where to start and what to register; it points
at these documents instead of restating them.

## The map

| Document | Open it when |
| --- | --- |
| [`reference/standards.md`](reference/standards.md) | You need a product norm — bit-exactness, determinism, bit-pattern transport, layers, errors, panics, caller state, profile ownership, the integration topology, the portability floor — or the test that establishes it |
| [`reference/architecture.md`](reference/architecture.md) | Changing a layer boundary or the encoding flow |
| [`reference/domain-model.md`](reference/domain-model.md) | Naming anything: the vocabulary is normative |
| [`reference/profiles.md`](reference/profiles.md) | Touching profile data, profile selection, or its provenance |
| [`reference/public-interface.md`](reference/public-interface.md) | Touching package exports, the `encode` API, or the CLI |
| [`guides/usage.md`](guides/usage.md) | Installing the encoder, encoding a file, calling it from each language |
| [`guides/development.md`](guides/development.md) | Building, running the tests, and the rules the work is held to |
| [`findings/2ch-byte-exactness.md`](findings/2ch-byte-exactness.md) | Needing the 2ch evidence, its root causes, its retractions, or its trust boundary |
| [`findings/concurrency-curves.md`](findings/concurrency-curves.md) | Asking how many encodes this machine runs at once, whether the kernel's internal parallelism pays for itself, or how to size a worker pool |
| [`findings/browser-shell-toolchain-options.md`](findings/browser-shell-toolchain-options.md) | Asking whether the browser shell should stay Rust and what C, Zig or MoonBit would actually buy |
| [`methodology/byte-exact-diagnosis.md`](methodology/byte-exact-diagnosis.md) | Chasing any byte difference against an external build |
| [`roadmap.md`](roadmap.md) | Asking what is still open |

## How this tree is organized

- **`reference/`** describes how the system *is*; a change here is a change to
  what the code and the tests are expected to do. `standards.md` holds the
  product norms, and the topical documents hold the design of one area.
- **`guides/`** holds task instructions and the rules the work is held to. A
  working rule lives in `development.md`, not in a reference document.
- **`findings/`** holds evidence for a completed result: the observation behind
  each root cause, the accounting that closes, the retractions, and the boundary
  of what the corpus proves. A finding is history — it records what was diagnosed
  at a point in time, and it links to reference material instead of restating it.
- **`methodology/`** holds reusable method, written only after it has actually
  decided a case.
- **`figures/`** holds committed figures and the recorded samples behind them:
  a generated image and the data it was rendered from, so a reader can check the
  numbers and re-render the picture instead of trusting the picture. Each is
  produced by a script under `scripts/` that is byte-stable, and the finding that
  discusses it embeds it by relative link.

## Conventions

- One claim, one link. Numbers live in the finding or in profile data, not in
  prose that can drift.
- A rule has one home: product norms in `reference/standards.md`, working rules
  in `guides/development.md` and the `AGENTS.md` set, area design in the topical
  reference documents. Everywhere else points at it.
- Relative links only, so the tree works from a checkout, a wheel, and a ZIP
  import alike.
- Anything a reader could act on goes in `guides/`; anything a reader could
  verify goes in `findings/`; anything a reader is expected to follow goes in
  `reference/` or `AGENTS.md`.
- Removing a mechanism, target, asset or public item means updating or deleting
  the documents that describe it in the same change and correcting this map;
  the checklist is in
  [`guides/development.md`](guides/development.md#adding-and-removing-files).
