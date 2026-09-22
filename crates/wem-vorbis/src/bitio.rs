//! Canonical LSB-first Vorbis bit reader and packer primitives
//! (Python: `wwise_wem/vorbis/bitio.py`).

use std::fmt;

/// Bitstream access errors (Python: `EOFError("out of bits")` / `ValueError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitError {
    /// The stream ran out of bits.
    OutOfBits,
    /// Write requested more than 32 bits.
    BitsOutOfRange { bits: u32 },
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitError::OutOfBits => write!(f, "out of bits"),
            BitError::BitsOutOfRange { bits } => write!(f, "bits must be 0..32, got {bits}"),
        }
    }
}

impl std::error::Error for BitError {}

/// LSB-first bit reader (Python `BitReader`).
#[derive(Debug)]
pub struct BitReader<'a> {
    data: &'a [u8],
    byte: usize,
    bit: u32,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte: 0,
            bit: 0,
        }
    }

    /// Read `n` bits (LSB first); `n` must be <= 64.
    pub fn read(&mut self, n: u32) -> Result<u64, BitError> {
        let mut v = 0u64;
        for i in 0..n {
            if self.byte >= self.data.len() {
                return Err(BitError::OutOfBits);
            }
            let b = (self.data[self.byte] >> self.bit) & 1;
            v |= (b as u64) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.byte += 1;
            }
        }
        Ok(v)
    }

    /// Current bit position.
    pub fn tell_bits(&self) -> u64 {
        (self.byte as u64) * 8 + self.bit as u64
    }

    /// Remaining bits.
    pub fn bits_left(&self) -> u64 {
        (self.data.len() as u64) * 8 - self.tell_bits()
    }
}

/// LSB-first bit packer (Python `OggPack`).
///
/// Bits are accumulated in a u64 (LSB-first) and flushed as whole bytes the
/// moment 8 bits are ready; the emitted byte stream is bit-identical to the
/// previous per-write C store, while removing the per-write branch ladder.
#[derive(Debug)]
pub struct OggPack {
    buffer: Vec<u8>,
    /// Pending bits, LSB-first. Invariant: every bit at or above `accbits`
    /// is zero, so the bytes [`OggPack::get_buffer`] reads back are exactly
    /// the bits written and nothing else.
    acc: u64,
    /// Valid bit count in `acc` (always < 8 after a flush).
    accbits: u32,
}

impl OggPack {
    /// Reserve space for approximately `initial` output bytes.
    pub fn new(initial: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(initial.max(1)),
            acc: 0,
            accbits: 0,
        }
    }

    /// Write the low `bits` (0..=32) of `value` LSB-first.
    ///
    /// Only the low `bits` of `value` reach the stream: a value with anything
    /// set above them is truncated to those bits, not rejected. That is the
    /// primitive's defined behaviour rather than a convenience — libogg's
    /// `oggpack_write` is `value &= mask[bits]` (`mask[32] == 0xffffffff`),
    /// and the reference port this crate mirrors does the same
    /// (`v = value & MASKS[bits]`, "C byte store truncates"). Callers own the
    /// domain of what they write: an encoder path checks its own ranges
    /// before writing (a width outside `0..=32` is
    /// [`BitError::BitsOutOfRange`], an unrepresentable codeword length is a
    /// codebook error), so the mask is a no-op for valid input. It is applied
    /// at every width including 32: an unmasked value there would sit above
    /// the accumulator's valid-bit count and come back out in the bytes
    /// written *after* it.
    pub fn write(&mut self, value: u64, bits: u32) -> Result<(), BitError> {
        if bits > 32 {
            return Err(BitError::BitsOutOfRange { bits });
        }
        if bits == 0 {
            return Ok(());
        }
        // `1u64 << 32` is representable in u64, so one mask covers 0..=32.
        let v = value & ((1u64 << bits) - 1);
        self.acc |= v << self.accbits;
        self.accbits += bits;
        while self.accbits >= 8 {
            self.buffer.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.accbits -= 8;
        }
        Ok(())
    }

    /// Return the packed bytes.
    ///
    /// Pending bits (the final partial byte) live in the accumulator, not the
    /// buffer, so they are appended here instead of read back from storage.
    pub fn get_buffer(&self) -> Vec<u8> {
        let mut out = self.buffer.clone();
        if self.accbits > 0 {
            out.push((self.acc & 0xFF) as u8);
        }
        out
    }

    /// Finish packing and return the owned byte buffer without copying it.
    pub fn into_buffer(mut self) -> Vec<u8> {
        if self.accbits > 0 {
            self.buffer.push((self.acc & 0xFF) as u8);
        }
        self.buffer
    }

    /// Reset to the start of the buffer (Python `reset`).
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.acc = 0;
        self.accbits = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_roundtrip_lsb_first() {
        let mut pack = OggPack::new(64);
        // 5-bit value 13 (0b01101), then 3-bit value 6 (0b110).
        pack.write(13, 5).unwrap();
        pack.write(6, 3).unwrap();
        let bytes = pack.get_buffer();
        let mut reader = BitReader::new(&bytes);
        assert_eq!(reader.read(5).unwrap(), 13);
        assert_eq!(reader.read(3).unwrap(), 6);
        assert_eq!(reader.bits_left(), 0);
    }

    #[test]
    fn out_of_bits_raises() {
        // 0b0011_1100: bits LSB-first = 0,0,1,1,1,1,0,0
        let mut reader = BitReader::new(&[0b0011_1100]);
        assert_eq!(reader.read(5).unwrap(), 0b11100); // bits 0..4
        assert_eq!(reader.read(3).unwrap(), 0b001); // bits 5..7 (1,0,0)
        assert_eq!(reader.read(1).unwrap_err(), BitError::OutOfBits);
    }

    #[test]
    fn write_truncates_to_the_requested_width() {
        // Masking, not rejection, is the primitive's behaviour: libogg's
        // `oggpack_write` is `value &= mask[bits]` and the reference port
        // mirrors it. Values here are the ones the reference `OggPack`
        // produces for the same calls.
        let mut pack = OggPack::new(8);
        pack.write(0x1FF, 8).unwrap();
        assert_eq!(pack.get_buffer(), vec![0xFF]);

        let mut wide = OggPack::new(8);
        wide.write(0xFFFF_FFFF, 32).unwrap();
        wide.write(0xAB, 8).unwrap();
        assert_eq!(wide.get_buffer(), vec![0xFF, 0xFF, 0xFF, 0xFF, 0xAB]);
    }

    #[test]
    fn write_masks_at_width_32_so_nothing_leaks_into_later_bytes() {
        // A value with bits above 32 set must not survive the write: unmasked
        // it would stay in the accumulator above the valid-bit count and come
        // back out of the bytes written after it. Reference output for the
        // same three writes is six zero bytes.
        let mut pack = OggPack::new(16);
        pack.write(1u64 << 40, 32).unwrap();
        pack.write(0, 8).unwrap();
        pack.write(0, 8).unwrap();
        assert_eq!(pack.get_buffer(), vec![0u8; 6]);
    }

    #[test]
    fn write_rejects_a_width_the_type_cannot_carry() {
        let mut pack = OggPack::new(8);
        assert_eq!(
            pack.write(0, 33).unwrap_err(),
            BitError::BitsOutOfRange { bits: 33 }
        );
        assert!(pack.get_buffer().is_empty());
    }
}
