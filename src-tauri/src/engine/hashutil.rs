//! 純 Rust SHA-256（避免多拉相依）。更新檔校驗與共享翻譯記憶的鍵都用它。

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
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

fn compress(h: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for (i, wi) in w.iter_mut().take(16).enumerate() {
        *wi = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let mut v = *h;
    for i in 0..64 {
        let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
        let ch = (v[4] & v[5]) ^ ((!v[4]) & v[6]);
        let t1 = v[7]
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
        let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
        let t2 = s0.wrapping_add(maj);
        v[7] = v[6];
        v[6] = v[5];
        v[5] = v[4];
        v[4] = v[3].wrapping_add(t1);
        v[3] = v[2];
        v[2] = v[1];
        v[1] = v[0];
        v[0] = t1.wrapping_add(t2);
    }
    for i in 0..8 {
        h[i] = h[i].wrapping_add(v[i]);
    }
}

pub struct Sha256Hasher {
    h: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    bit_len: u64,
}

impl Sha256Hasher {
    pub fn new() -> Self {
        Self {
            h: H0,
            buf: [0; 64],
            buf_len: 0,
            bit_len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut rest = data;
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            let take = need.min(rest.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&rest[..take]);
            self.buf_len += take;
            self.bit_len += (take as u64) * 8;
            rest = &rest[take..];
            if self.buf_len == 64 {
                compress(&mut self.h, &self.buf);
                self.buf_len = 0;
            }
        }
        while rest.len() >= 64 {
            compress(&mut self.h, &rest[..64]);
            self.bit_len += 512;
            rest = &rest[64..];
        }
        if !rest.is_empty() {
            self.buf[..rest.len()].copy_from_slice(rest);
            self.buf_len = rest.len();
            self.bit_len += (rest.len() as u64) * 8;
        }
    }

    pub fn finalize_hex(mut self) -> String {
        let bit_len = self.bit_len;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            for i in self.buf_len..64 {
                self.buf[i] = 0;
            }
            compress(&mut self.h, &self.buf);
            self.buf = [0; 64];
            self.buf_len = 0;
        }
        for i in self.buf_len..56 {
            self.buf[i] = 0;
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        compress(&mut self.h, &self.buf);
        self.h.iter().map(|x| format!("{x:08x}")).collect()
    }
}

/// 回傳 64 字元小寫 hex。
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256Hasher::new();
    h.update(data);
    h.finalize_hex()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut s = Sha256Hasher::new();
        s.update(b"ab");
        s.update(b"c");
        assert_eq!(
            s.finalize_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
