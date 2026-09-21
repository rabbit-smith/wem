# Python: encode a WAV file

Install the package, then run the example from the repository root:

```sh
pip install -e .
python3 examples/python/encode_wav.py tests/fixtures/input.wav out.wem
```

`encode` reads the RIFF header and automatically selects an installed profile
for the WAV geometry. Pass `--profile wwise2013-2ch-48000` only when the input
geometry is already known and you want to assert the choice.
