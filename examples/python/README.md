# Python: encode a WAV file

Install the package, then run the example from the repository root:

```sh
pip install -e .
python3 examples/python/encode_wav.py tests/fixtures/input.wav out.wem
```

`encode` reads the RIFF header and selects the installed configuration for that
geometry. Pass `--wwise-version 2013` (or `2013.2`) only when you want to name
the Wwise generation explicitly; the geometry still comes from the input, and
the two together are the whole selection.
