//! minimal crypto primitives needed by the websocket protocol.
//!
//! the handshake needs exactly one hash (`sha1(key ++ GUID)`), two fixed size base64
//! encodings and a few random bytes per frame. depending on the `sha1`, `base64` and
//! `rand` crates for this pulls the whole RustCrypto `digest` stack plus a userspace
//! CSPRNG into the tree, so they are implemented here instead. only `getrandom` is kept
//! as it is the portable interface to the operating system entropy source.

/// SHA-1 as specified by [RFC 3174].
///
/// only used to derive the `Sec-WebSocket-Accept` header from a client offered key. that
/// derivation is a fixed protocol handshake and carries no secrecy requirement of its own,
/// SHA-1's collision weakness is not relevant to it.
///
/// [RFC 3174]: https://www.rfc-editor.org/rfc/rfc3174
pub(crate) struct Sha1 {
    state: [u32; 5],
    /// buffer for the current partial 64 byte block.
    block: [u8; 64],
    /// total message length in bytes. doubles as the index into `block` via `% 64`.
    len: u64,
}

impl Sha1 {
    pub(crate) const fn new() -> Self {
        Self {
            state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            block: [0; 64],
            len: 0,
        }
    }

    pub(crate) fn update(&mut self, mut input: &[u8]) {
        let mut idx = (self.len % 64) as usize;
        self.len += input.len() as u64;

        // top off the partial block first. bail out early when it is still not full.
        if idx > 0 {
            let n = core::cmp::min(64 - idx, input.len());
            self.block[idx..idx + n].copy_from_slice(&input[..n]);
            input = &input[n..];
            idx += n;
            if idx < 64 {
                return;
            }
            let block = self.block;
            self.compress(&block);
        }

        let (blocks, rem) = input.as_chunks::<64>();
        for block in blocks {
            self.compress(block);
        }

        self.block[..rem.len()].copy_from_slice(rem);
    }

    pub(crate) fn finalize(mut self) -> [u8; 20] {
        // message is padded with a 0x80 byte, zeroes up to 56 bytes mod 64 and the big
        // endian bit length. worst case padding is 63 + 8 bytes so 72 is always enough.
        let bit_len = self.len * 8;
        let idx = (self.len % 64) as usize;
        let pad_len = if idx < 56 { 56 - idx } else { 120 - idx };

        let mut pad = [0; 72];
        pad[0] = 0x80;
        pad[pad_len..pad_len + 8].copy_from_slice(&bit_len.to_be_bytes());
        self.update(&pad[..pad_len + 8]);

        debug_assert_eq!(self.len % 64, 0, "padded message must be block aligned");

        let mut out = [0; 20];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (word, chunk) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*chunk);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = self.state;

        for (i, &w) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | (!b & d), 0x5A827999),
                20..40 => (b ^ c ^ d, 0x6ED9EBA1),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }

        for (state, add) in self.state.iter_mut().zip([a, b, c, d, e]) {
            *state = state.wrapping_add(add);
        }
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// standard base64 alphabet with padding. `OUT` must be the exact encoded length of `IN`,
/// which is checked at compile time.
pub(crate) fn base64<const IN: usize, const OUT: usize>(input: &[u8; IN]) -> [u8; OUT] {
    const {
        assert!(OUT == IN.div_ceil(3) * 4, "OUT must be the base64 encoded length of IN");
    }

    /// index the alphabet with the `idx`th six bit group of `n`, counting from the top of
    /// the 24 bit group.
    const fn sextet(n: u32, idx: u32) -> u8 {
        B64[(n >> (18 - idx * 6)) as usize & 0x3f]
    }

    // trailing bytes of a non multiple of 3 input keep the '=' they are initialized with.
    let mut out = [b'='; OUT];

    // every whole three byte group becomes four characters. the const assert above
    // guarantees `out` divides evenly, so `quads` has no remainder of its own.
    let (groups, rem) = input.as_chunks::<3>();
    let (quads, _) = out.as_chunks_mut::<4>();

    for (group, quad) in groups.iter().zip(&mut *quads) {
        let n = u32::from_be_bytes([0, group[0], group[1], group[2]]);
        *quad = [sextet(n, 0), sextet(n, 1), sextet(n, 2), sextet(n, 3)];
    }

    // a trailing one or two bytes encode to two or three characters, the rest stays '='.
    if let Some(quad) = quads.get_mut(groups.len()) {
        match *rem {
            [a] => {
                let n = u32::from_be_bytes([0, a, 0, 0]);
                [quad[0], quad[1]] = [sextet(n, 0), sextet(n, 1)];
            }
            [a, b] => {
                let n = u32::from_be_bytes([0, a, b, 0]);
                [quad[0], quad[1], quad[2]] = [sextet(n, 0), sextet(n, 1), sextet(n, 2)];
            }
            _ => {}
        }
    }

    out
}

/// size of the thread local entropy pool. kept small so few unused random bytes are
/// resident at any time.
const POOL: usize = 256;

/// buffered operating system entropy and the offset of the first unused byte.
struct Pool {
    buf: [u8; POOL],
    pos: usize,
}

std::thread_local! {
    /// starts fully consumed so the first draw refills from the OS.
    static POOLED: core::cell::RefCell<Pool> = const {
        core::cell::RefCell::new(Pool { buf: [0; POOL], pos: POOL })
    };
}

/// `N` random bytes from the operating system CSPRNG.
///
/// RFC6455 requires the per frame masking key to be unpredictable, so a weaker userspace
/// generator is not an option. the key is only 4 bytes though, so bytes are drawn from the
/// OS in `POOL` sized batches to keep a syscall off the per frame path. the bytes handed
/// out are OS CSPRNG output either way, batching only amortizes the call.
pub(crate) fn random<const N: usize>() -> [u8; N] {
    const {
        assert!(N <= POOL, "requested more random bytes than the pool holds");
    }

    POOLED.with_borrow_mut(|pool| {
        if pool.pos + N > POOL {
            getrandom::fill(&mut pool.buf).expect("operating system entropy source is unavailable");
            pool.pos = 0;
        }

        let taken = &mut pool.buf[pool.pos..pool.pos + N];
        let mut out = [0; N];
        out.copy_from_slice(taken);
        // consumed bytes go on the wire, but clear them so they do not linger in the pool.
        taken.fill(0);
        pool.pos += N;
        out
    })
}

#[cfg(test)]
mod test {
    use super::*;

    fn sha1(input: &[u8]) -> [u8; 20] {
        let mut hasher = Sha1::new();
        hasher.update(input);
        hasher.finalize()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha1_rfc3174_vectors() {
        // test vectors from RFC 3174 section 7.3.
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            hex(&sha1(&[b'a'; 1_000_000])),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
        assert_eq!(
            hex(&sha1(
                &b"0123456701234567012345670123456701234567012345670123456701234567".repeat(10)
            )),
            "dea356a2cddd90c7a7ecedc5ebb563934f460452"
        );
    }

    #[test]
    fn sha1_empty() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    /// every length around the 56/64 byte padding boundaries must agree with a single
    /// shot hash, and splitting the same input at any point must not change the digest.
    #[test]
    fn sha1_block_boundaries_and_split_updates() {
        let data = (0..200u32).map(|i| i as u8).collect::<Vec<_>>();

        // reference digests generated for each prefix length by the one shot path.
        for len in 0..data.len() {
            let input = &data[..len];
            let expected = sha1(input);

            for split in 0..=len {
                let mut hasher = Sha1::new();
                hasher.update(&input[..split]);
                hasher.update(&input[split..]);
                assert_eq!(hasher.finalize(), expected, "len {len} split at {split}");
            }
        }
    }

    #[test]
    fn sha1_many_small_updates() {
        let data = (0..500u32).map(|i| i as u8).collect::<Vec<_>>();
        let mut hasher = Sha1::new();
        for byte in &data {
            hasher.update(&[*byte]);
        }
        assert_eq!(hasher.finalize(), sha1(&data));
    }

    #[test]
    fn base64_rfc4648_vectors() {
        // test vectors from RFC 4648 section 10. one per input length mod 3.
        assert_eq!(&base64::<1, 4>(b"f"), b"Zg==");
        assert_eq!(&base64::<2, 4>(b"fo"), b"Zm8=");
        assert_eq!(&base64::<3, 4>(b"foo"), b"Zm9v");
        assert_eq!(&base64::<4, 8>(b"foob"), b"Zm9vYg==");
        assert_eq!(&base64::<5, 8>(b"fooba"), b"Zm9vYmE=");
        assert_eq!(&base64::<6, 8>(b"foobar"), b"Zm9vYmFy");
    }

    /// the two sizes the handshake actually uses: a 20 byte sha1 digest and a 16 byte nonce.
    #[test]
    fn base64_handshake_sizes() {
        assert_eq!(&base64::<20, 28>(&[0xff; 20]), b"//////////////////////////8=");
        assert_eq!(&base64::<16, 24>(&[0; 16]), b"AAAAAAAAAAAAAAAAAAAAAA==");
    }

    /// exercises the whole alphabet including the `+` and `/` characters.
    #[test]
    fn base64_full_alphabet() {
        let mut input = [0u8; 48];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 5 + i / 3) as u8;
        }
        let encoded = base64::<48, 64>(&input);
        assert!(encoded.iter().all(|b| B64.contains(b)));
    }

    #[test]
    fn random_is_not_constant() {
        // draws far more than POOL bytes to cover the refill path.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..500 {
            seen.insert(random::<4>());
        }
        assert!(seen.len() > 400, "masking keys repeat far too often: {}", seen.len());

        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            seen.insert(random::<16>());
        }
        assert_eq!(seen.len(), 100);
    }
}
