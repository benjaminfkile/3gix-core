//! SHA-256 (FIPS 180-4), written out with integer operations only.
//!
//! Section addresses, module hashes, source tile hashes, and source manifest
//! hashes are all SHA-256 (`client-compute.md` sections 2.1, 3, and 5). The
//! library owns one implementation so the hub, the worker, the modules, and
//! the conformance vectors hash with the same code on every target. It uses
//! no platform intrinsics, so native and WebAssembly builds agree by
//! construction.

/// Length of a SHA-256 digest in bytes.
pub const DIGEST_LEN: usize = 32;

/// Round constants: the first 32 bits of the fractional parts of the cube
/// roots of the first 64 primes.
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Initial hash value: the first 32 bits of the fractional parts of the
/// square roots of the first 8 primes.
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// An incremental SHA-256 hasher: feed bytes with [`Sha256::update`], then
/// take the digest with [`Sha256::finish`].
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// A hasher that has seen no bytes.
    pub fn new() -> Self {
        Sha256 {
            state: H0,
            block: [0; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    /// Feeds `data` to the hasher.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        if self.block_len > 0 {
            let take = (64 - self.block_len).min(data.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&data[..take]);
            self.block_len += take;
            data = &data[take..];
            if self.block_len < 64 {
                return;
            }
            let block = self.block;
            compress(&mut self.state, &block);
            self.block_len = 0;
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for b in blocks {
            compress(&mut self.state, b);
        }
        self.block[..rest.len()].copy_from_slice(rest);
        self.block_len = rest.len();
    }

    /// Pads the message and returns the 32-byte digest.
    pub fn finish(mut self) -> [u8; DIGEST_LEN] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.block_len != 56 {
            self.update(&[0]);
        }
        self.update(&bit_len.to_be_bytes());
        debug_assert_eq!(self.block_len, 0);
        let mut out = [0u8; DIGEST_LEN];
        for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            o.copy_from_slice(&s.to_be_bytes());
        }
        out
    }
}

/// The SHA-256 digest of `data`.
pub fn sha256(data: &[u8]) -> [u8; DIGEST_LEN] {
    let mut h = Sha256::new();
    h.update(data);
    h.finish()
}

/// Lowercase hexadecimal of `bytes`, two characters per byte.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    s
}

/// Parses exactly 64 hexadecimal characters (either case) into a 32-byte
/// digest. `None` for any other input.
pub fn from_hex(s: &str) -> Option<[u8; DIGEST_LEN]> {
    let b = s.as_bytes();
    if b.len() != 2 * DIGEST_LEN {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; DIGEST_LEN];
    for (o, pair) in out.iter_mut().zip(b.as_chunks::<2>().0) {
        *o = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(out)
}

/// The SHA-256 compression function over one 64-byte block.
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (wi, c) in w.iter_mut().zip(block.as_chunks::<4>().0) {
        *wi = u32::from_be_bytes(*c);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *s = s.wrapping_add(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        // FIPS 180-4 examples and the empty message.
        let cases: [(&[u8], &str); 3] = [
            (
                b"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (msg, want) in cases {
            assert_eq!(to_hex(&sha256(msg)), want);
        }
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            to_hex(&sha256(&million)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn incremental_matches_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 + 3) as u8).collect();
        let want = sha256(&data);
        for split in [0, 1, 55, 56, 63, 64, 65, 127, 128, 999, 1000] {
            let mut h = Sha256::new();
            h.update(&data[..split]);
            h.update(&data[split..]);
            assert_eq!(h.finish(), want, "split {split}");
        }
        let mut h = Sha256::new();
        for b in &data {
            h.update(core::slice::from_ref(b));
        }
        assert_eq!(h.finish(), want);
    }

    #[test]
    fn hex_round_trip() {
        let d = sha256(b"abc");
        let s = to_hex(&d);
        assert_eq!(from_hex(&s), Some(d));
        assert_eq!(from_hex(&s.to_uppercase()), Some(d));
        assert_eq!(from_hex(&s[1..]), None);
        assert_eq!(from_hex(&format!("{}g", &s[1..])), None);
        assert_eq!(to_hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }
}
