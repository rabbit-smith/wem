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

/// LSB-first bit packer mirroring the x86 struct layout (Python `OggPack`).
#[derive(Debug)]
pub struct OggPack {
    buffer: Vec<u8>,
    ptr: usize,
    endbyte: usize,
    endbit: u32,
}

impl OggPack {
    /// Allocate `initial` bytes of zeroed storage.
    pub fn new(initial: usize) -> Self {
        let mut buffer = vec![0u8; initial.max(1)];
        buffer[0] = 0;
        Self {
            buffer,
            ptr: 0,
            endbyte: 0,
            endbit: 0,
        }
    }

    /// Write the low `bits` (0..=32) of `value` LSB-first.
    pub fn write(&mut self, value: u64, bits: u32) -> Result<(), BitError> {
        if bits > 32 {
            return Err(BitError::BitsOutOfRange { bits });
        }
        if self.endbyte + 4 >= self.buffer.len() {
            self.buffer.resize(self.buffer.len() + 256, 0);
        }
        let mask = if bits == 32 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let v = value & mask;
        let endbit = self.endbit;
        // C byte store truncates: *ptr |= value << endbit
        self.buffer[self.ptr] |= (v << endbit) as u8;
        let total = endbit + bits;
        if total >= 8 {
            self.buffer[self.ptr + 1] = (v >> (8 - endbit)) as u8;
            if total >= 16 {
                self.buffer[self.ptr + 2] = (v >> (16 - endbit)) as u8;
                if total >= 24 {
                    self.buffer[self.ptr + 3] = (v >> (24 - endbit)) as u8;
                    if total >= 32 {
                        if endbit > 0 {
                            self.buffer[self.ptr + 4] = (v >> (32 - endbit)) as u8;
                        } else {
                            self.buffer[self.ptr + 4] = 0;
                        }
                    }
                }
            }
        }
        let add = (total / 8) as usize;
        self.endbyte += add;
        self.ptr += add;
        self.endbit = total % 8;
        Ok(())
    }

    /// Bytes consumed so far (rounded up).
    pub fn bytes_used(&self) -> usize {
        self.endbyte + (self.endbit + 7) as usize / 8
    }

    /// Return the packed bytes.
    pub fn get_buffer(&self) -> Vec<u8> {
        self.buffer[..self.bytes_used()].to_vec()
    }

    /// Reset to the start of the buffer (Python `reset`).
    pub fn reset(&mut self) {
        self.ptr = 0;
        self.endbyte = 0;
        self.endbit = 0;
        if !self.buffer.is_empty() {
            self.buffer[0] = 0;
        }
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
}
