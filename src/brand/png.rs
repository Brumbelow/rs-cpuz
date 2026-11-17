//! A minimal PNG writer for the build script: 8-bit RGBA, no dependencies.
//!
//! The compressor only emits run-length matches, which is all the flat-coloured
//! icon artwork needs once each row is stored as its difference to the row above.

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Longest match DEFLATE can express.
const MAX_RUN: usize = 258;

/// Encodes `rgba` (4 bytes per pixel, rows top to bottom) as a PNG file.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // Bit depth 8, colour type 6 (RGBA), default compression, filtering and no interlacing.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut out = SIGNATURE.to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib(&filter_up(rgba, width as usize * 4)));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let checksum = crc32(&out[start..]);
    out.extend_from_slice(&checksum.to_be_bytes());
}

/// Applies PNG filter type 2 ("Up") to every scanline of `stride` bytes.
fn filter_up(pixels: &[u8], stride: usize) -> Vec<u8> {
    if stride == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(pixels.len() + pixels.len() / stride);
    let mut above = vec![0u8; stride];
    for row in pixels.chunks_exact(stride) {
        out.push(2);
        out.extend(row.iter().zip(&above).map(|(&byte, &up)| byte.wrapping_sub(up)));
        above.copy_from_slice(row);
    }
    out
}

/// Wraps `data` in a zlib stream holding one fixed-Huffman DEFLATE block.
fn zlib(data: &[u8]) -> Vec<u8> {
    let mut bits = BitWriter { out: vec![0x78, 0x01], pending: 0, count: 0 };
    // Final block, fixed Huffman codes.
    bits.put(1, 1);
    bits.put(1, 2);
    let mut pos = 0;
    while pos < data.len() {
        let run = if pos == 0 { 0 } else { run_length(&data[pos..], data[pos - 1]) };
        if run >= 3 {
            bits.run(run);
            pos += run;
        } else {
            bits.symbol(u32::from(data[pos]));
            pos += 1;
        }
    }
    bits.symbol(256);
    let mut out = bits.finish();
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Number of leading bytes of `data` equal to `byte`, capped at the longest match.
fn run_length(data: &[u8], byte: u8) -> usize {
    data.iter().take(MAX_RUN).take_while(|&&b| b == byte).count()
}

/// Packs DEFLATE's bit stream: least significant bit first.
struct BitWriter {
    out: Vec<u8>,
    pending: u32,
    count: u32,
}

impl BitWriter {
    fn put(&mut self, value: u32, bits: u32) {
        self.pending |= value << self.count;
        self.count += bits;
        while self.count >= 8 {
            self.out.push(self.pending as u8);
            self.pending >>= 8;
            self.count -= 8;
        }
    }

    /// Writes a Huffman code, which unlike every other field goes most significant bit first.
    fn code(&mut self, code: u32, bits: u32) {
        self.put(code.reverse_bits() >> (32 - bits), bits);
    }

    /// Writes a literal/length symbol with the fixed code of RFC 1951, section 3.2.6.
    fn symbol(&mut self, symbol: u32) {
        match symbol {
            0..=143 => self.code(0x30 + symbol, 8),
            144..=255 => self.code(0x190 + symbol - 144, 9),
            256..=279 => self.code(symbol - 256, 7),
            _ => self.code(0xC0 + symbol - 280, 8),
        }
    }

    /// Writes a match of `length` bytes (3..=258) at distance 1.
    fn run(&mut self, length: usize) {
        let offset = length as u32 - 3;
        if length == MAX_RUN {
            self.symbol(285);
        } else if offset < 8 {
            self.symbol(257 + offset);
        } else {
            // Above ten bytes each group of four codes carries `extra` more bits.
            let extra = offset.ilog2() - 2;
            self.symbol(261 + 4 * extra + (offset >> extra) - 4);
            self.put(offset & ((1 << extra) - 1), extra);
        }
        // Distance code 0 (a distance of one byte) in the fixed 5-bit code.
        self.code(0, 5);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.pending as u8);
        }
        self.out
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads DEFLATE's bit stream, least significant bit first.
    struct BitReader<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl BitReader<'_> {
        fn bit(&mut self) -> u32 {
            let bit = (self.data[self.pos / 8] >> (self.pos % 8)) & 1;
            self.pos += 1;
            u32::from(bit)
        }

        fn bits(&mut self, count: u32) -> u32 {
            (0..count).fold(0, |value, i| value | (self.bit() << i))
        }

        fn code(&mut self, count: u32) -> u32 {
            (0..count).fold(0, |value, _| (value << 1) | self.bit())
        }

        fn symbol(&mut self) -> u32 {
            let mut code = self.code(7);
            if code <= 0x17 {
                return 256 + code;
            }
            code = (code << 1) | self.bit();
            match code {
                0x30..=0xBF => code - 0x30,
                0xC0..=0xC7 => 280 + code - 0xC0,
                _ => 144 + ((code << 1) | self.bit()) - 0x190,
            }
        }
    }

    /// Length base values and extra-bit counts of symbols 257..=285 (RFC 1951, section 3.2.5).
    const LENGTHS: [(usize, u32); 29] = [
        (3, 0),
        (4, 0),
        (5, 0),
        (6, 0),
        (7, 0),
        (8, 0),
        (9, 0),
        (10, 0),
        (11, 1),
        (13, 1),
        (15, 1),
        (17, 1),
        (19, 2),
        (23, 2),
        (27, 2),
        (31, 2),
        (35, 3),
        (43, 3),
        (51, 3),
        (59, 3),
        (67, 4),
        (83, 4),
        (99, 4),
        (115, 4),
        (131, 5),
        (163, 5),
        (195, 5),
        (227, 5),
        (258, 0),
    ];

    /// Decodes a zlib stream made of a single fixed-Huffman block using distance-1 matches.
    fn inflate(stream: &[u8]) -> Vec<u8> {
        assert_eq!(stream[..2], [0x78, 0x01]);
        let mut reader = BitReader { data: &stream[2..stream.len() - 4], pos: 0 };
        assert_eq!(reader.bits(1), 1, "final block");
        assert_eq!(reader.bits(2), 1, "fixed Huffman codes");
        let mut out = Vec::new();
        loop {
            match reader.symbol() {
                256 => break,
                literal @ 0..=255 => out.push(literal as u8),
                symbol => {
                    let (base, extra) = LENGTHS[symbol as usize - 257];
                    let length = base + reader.bits(extra) as usize;
                    assert_eq!(reader.code(5), 0, "distance code");
                    let last = *out.last().expect("a match needs a previous byte");
                    out.extend(std::iter::repeat(last).take(length));
                }
            }
        }
        assert_eq!(stream[stream.len() - 4..], adler32(&out).to_be_bytes());
        out
    }

    #[test]
    fn checksums_match_their_check_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(&[]), 1);
    }

    #[test]
    fn deflate_round_trips_every_run_length() {
        let mut data = Vec::new();
        for length in 1..=600usize {
            data.push((length % 251) as u8 + 1);
            data.extend(std::iter::repeat(0).take(length));
        }
        assert_eq!(inflate(&zlib(&data)), data);
        // Long runs must actually be compressed.
        assert!(zlib(&vec![7u8; 100_000]).len() < 1_000);
    }

    #[test]
    fn deflate_round_trips_literals() {
        let all: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).collect();
        assert_eq!(inflate(&zlib(&all)), all);
        assert_eq!(inflate(&zlib(&[])), Vec::<u8>::new());
        assert_eq!(inflate(&zlib(&[9, 9])), [9, 9]);
    }

    #[test]
    fn up_filter_stores_row_differences() {
        let pixels = [1, 2, 3, 4, 1, 2, 3, 4, 5, 2, 3, 0];
        assert_eq!(filter_up(&pixels, 4), [2, 1, 2, 3, 4, 2, 0, 0, 0, 0, 2, 4, 0, 0, 252]);
        assert!(filter_up(&[], 0).is_empty());
    }

    #[test]
    fn file_structure() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 1, 2, 3, 4];
        let png = encode(2, 2, &rgba);
        assert_eq!(png[..8], SIGNATURE);
        // IHDR: length 13, then width, height and the format bytes.
        assert_eq!(png[8..16], [0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        assert_eq!(png[16..29], [0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0, 0, 0]);
        assert_eq!(png[29..33], crc32(&png[12..29]).to_be_bytes());
        // IDAT holds the filtered scanlines.
        let length = u32::from_be_bytes([png[33], png[34], png[35], png[36]]) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        assert_eq!(inflate(&png[41..41 + length]), filter_up(&rgba, 8));
        assert_eq!(png[png.len() - 12..], [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
    }
}
