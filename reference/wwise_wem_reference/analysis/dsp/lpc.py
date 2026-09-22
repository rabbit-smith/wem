#!/usr/bin/env python3
"""Linear-prediction helpers for analysis-stream boundary synthesis."""
from __future__ import annotations

from typing import Sequence

from ..._f32 import _f32


def wwise_lpc_from_data(
    samples: Sequence[float], order: int = 16
) -> list[float]:
    """Return damped LPC coefficients using Durbin recursion.

    Autocorrelation and recursion use double precision; coefficients are
    stored as float32 with per-tap 0.99 damping for deterministic output.
    """
    if order < 1:
        raise ValueError("LPC order must be positive")
    if len(samples) <= order:
        raise ValueError("LPC input must be longer than its order")

    source = [float(value) for value in samples]
    autocorrelation = [
        sum(source[index] * source[index - lag] for index in range(lag, len(source)))
        for lag in range(order + 1)
    ]
    error = autocorrelation[0] * (1.0 + 1.0e-10)
    epsilon = autocorrelation[0] * 1.0e-9 + 1.0e-10
    coefficients = [0.0] * order

    for tap in range(order):
        if error < epsilon:
            break
        reflection = -autocorrelation[tap + 1]
        for previous in range(tap):
            reflection -= coefficients[previous] * autocorrelation[tap - previous]
        reflection /= error
        coefficients[tap] = reflection

        for previous in range(tap // 2):
            saved = coefficients[previous]
            coefficients[previous] = (
                coefficients[tap - previous - 1] * reflection + saved
            )
            coefficients[tap - previous - 1] = (
                saved * reflection + coefficients[tap - previous - 1]
            )
        if tap & 1:
            midpoint = tap // 2
            coefficients[midpoint] += coefficients[midpoint] * reflection
        error *= 1.0 - reflection * reflection

    damping = 0.99
    for tap in range(order):
        coefficients[tap] = _f32(coefficients[tap] * damping)
        damping *= 0.99
    return coefficients


def wwise_lpc_predict(
    coefficients: Sequence[float], prime: Sequence[float], count: int
) -> list[float]:
    """Predict ``count`` samples with a float32 work ring and recurrence."""
    order = len(coefficients)
    if order < 1:
        raise ValueError("LPC coefficients must not be empty")
    if len(prime) != order:
        raise ValueError("LPC prime length must equal coefficient order")
    if count < 0:
        raise ValueError("LPC prediction count must not be negative")

    work = [_f32(value) for value in prime] + [0.0] * count
    output: list[float] = []
    for index in range(count):
        prediction = 0.0
        for tap in range(order):
            prediction = _f32(
                prediction
                - work[index + tap] * float(coefficients[order - tap - 1])
            )
        work[order + index] = prediction
        output.append(prediction)
    return output


def wwise_first_frame_lpc_prime(
    source: Sequence[float],
    *,
    prefill: int = 128,
    batch: int = 4096,
    order: int = 16,
) -> list[float]:
    """Build the initial left-half history for the first analysis frame.

    A reversed input batch trains the LPC model; predictions over the zero
    prefix are reversed back into forward-time history.
    """
    if prefill < 1:
        raise ValueError("priming prefill must be positive")
    if batch <= order:
        raise ValueError("priming batch must be longer than LPC order")
    if len(source) < batch:
        raise ValueError("source does not contain the first priming batch")

    reversed_buffer = list(reversed([0.0] * prefill + [
        _f32(value) for value in source[:batch]
    ]))
    coefficients = wwise_lpc_from_data(reversed_buffer[:batch], order)
    predicted_reversed = wwise_lpc_predict(
        coefficients,
        reversed_buffer[batch - order : batch],
        prefill,
    )
    return list(reversed(predicted_reversed))
