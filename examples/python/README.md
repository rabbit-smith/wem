# Python: encode a WAV file, decode a WEM file

Install the package, then run the examples from the repository root:

```sh
pip install -e .
python3 examples/python/encode_wav.py tests/fixtures/input.wav out.wem
python3 examples/python/decode_wem.py tests/fixtures/reference.wem out.f32
```

`encode` reads the RIFF header and selects the installed configuration for that
geometry. Pass `--wwise-version 2013` (or `2013.2`) only when you want to name
the Wwise generation explicitly; the geometry still comes from the input, and
the two together are the whole selection.

`decode` takes no selection: a WEM is self-describing, and `result.channels`,
`result.sample_rate` and `result.total_frames` are the container's own
declaration, readable before the first block. Each iteration step is one bounded
block of interleaved f32 samples at ±1.0 full scale.

`decode_wem.py` writes those samples raw — interleaved little-endian f32, no
header — so the file is the decoder's output and not a conversion of it: no
sample format is invented, and no WAV writer that could be wrong in a way that
still looks plausible is shipped here. The printed line states the geometry and
the frame count the container declares, which is what makes the file
interpretable; `shasum -a 256 out.f32` (or `cmp` against another example's
output for the same WEM) is the check.