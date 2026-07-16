"""Canonical LSB-first Vorbis bit reader and packer primitives."""

from __future__ import annotations


class BitReader:
    def __init__(self, data: bytes):
        self.data = data
        self.byte = 0
        self.bit = 0

    def read(self, n: int) -> int:
        v = 0
        for i in range(n):
            if self.byte >= len(self.data):
                raise EOFError("out of bits")
            b = (self.data[self.byte] >> self.bit) & 1
            v |= b << i
            self.bit += 1
            if self.bit == 8:
                self.bit = 0
                self.byte += 1
        return v

    def tell_bits(self) -> int:
        return self.byte * 8 + self.bit

    def bits_left(self) -> int:
        return len(self.data) * 8 - self.tell_bits()


MASKS = [0]
for _i in range(1, 33):
    MASKS.append((1 << _i) - 1)


class OggPack:
    """Mirrors x86 struct: endbyte, endbit, buffer, ptr, storage."""

    def __init__(self, initial: int = 256) -> None:
        self.endbyte = 0
        self.endbit = 0
        self.buffer = bytearray(initial)
        self.buffer[0] = 0
        self.storage = initial
        self._ptr = 0

    def write(self, value: int, bits: int) -> None:
        if bits < 0 or bits > 32:
            raise ValueError("bits must be 0..32")
        if self.endbyte + 4 >= self.storage:
            grow = 256
            self.buffer.extend(b"\x00" * grow)
            self.storage += grow
        v = value & MASKS[bits]
        endbit = self.endbit
        # C byte store truncates: *ptr |= value << endbit
        self.buffer[self._ptr] = (self.buffer[self._ptr] | (v << endbit)) & 0xFF
        total = endbit + bits
        if total >= 8:
            self.buffer[self._ptr + 1] = (v >> (8 - endbit)) & 0xFF
            if total >= 16:
                self.buffer[self._ptr + 2] = (v >> (16 - endbit)) & 0xFF
                if total >= 24:
                    self.buffer[self._ptr + 3] = (v >> (24 - endbit)) & 0xFF
                    if total >= 32:
                        if endbit:
                            self.buffer[self._ptr + 4] = (v >> (32 - endbit)) & 0xFF
                        else:
                            self.buffer[self._ptr + 4] = 0
        add = total // 8
        self.endbyte += add
        self._ptr += add
        self.endbit = total & 7

    def bytes_used(self) -> int:
        return self.endbyte + (self.endbit + 7) // 8

    def get_buffer(self) -> bytes:
        return bytes(self.buffer[: self.bytes_used()])

    def reset(self) -> None:
        self._ptr = 0
        self.endbyte = 0
        self.endbit = 0
        if self.storage:
            self.buffer[0] = 0


__all__ = ["BitReader", "MASKS", "OggPack"]
