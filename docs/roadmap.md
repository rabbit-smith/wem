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
- Decide whether the browser shell can drop wasm-bindgen. The larger question is
  answered — C, Zig and MoonBit would bring a second module and a second runtime
  whose stack aliases the kernel's, for a binding layer that is under 3 kB of a
  module that is 71% compiled-in profile data
  ([options and measurements](findings/browser-shell-toolchain-options.md)).
  What is genuinely open is one number: how much of the 476,588-byte code section
  is wasm-bindgen. Building `crates/wem-wasm` with and without it at the same
  profile settles whether that route wins, washes, or loses.

A closed item leaves this page: its result and its evidence go into the finding
that produced it, and its claims into
[`reference/standards.md`](reference/standards.md#what-is-established).
