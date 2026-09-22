# Roadmap and outstanding work

Both registered profiles are byte-exact for their paired reference inputs. What
that means, and which test establishes it, is in
[`reference/standards.md`](reference/standards.md#what-is-established); the 2ch
evidence, its corpora and its limits are in
[`findings/2ch-byte-exactness.md`](findings/2ch-byte-exactness.md).

This page holds only what is still open.

## Open work

- Extend the independent real-build evidence with varied-duration, long-form
  material, while retaining the deterministic synthetic edge and differential
  coverage. Today's boundary is one 96,000-frame paired input and eight
  48,000-frame representative and stress inputs; boundary lengths are covered by
  native/oracle agreement and the fuzz comparison only
  ([trust boundary](findings/2ch-byte-exactness.md#trust-boundary)).

A closed item leaves this page: its result and its evidence go into the finding
that produced it, and its claims into
[`reference/standards.md`](reference/standards.md#what-is-established).
