"""Profile identity used by the encoder application.

The profile *values* live in the kernel as Rust constants; what this package
holds is the identity type (:mod:`wwise_wem.profiles.key`) plus the label rule
derived from it. There is no loader here, because there is no resource tree to
load: a caller selects with ``WwiseProfile`` and the kernel resolves it.
"""

__all__: list[str] = []
