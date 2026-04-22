const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const PRIME: u64 = 0x100000001b3;

/// FNV-1a 64-bit hash function.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hasher = Fnv1aHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// Streaming FNV-1a 64-bit hasher for incremental hashing without allocation.
pub struct Fnv1aHasher {
    hash: u64,
}

impl Fnv1aHasher {
    pub fn new() -> Self {
        Self {
            hash: OFFSET_BASIS,
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(PRIME);
        }
    }

    pub fn finish(&self) -> u64 {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_produce_deterministic_hash() {
        let a = fnv1a_64(b"hello world");
        let b = fnv1a_64(b"hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn should_produce_different_hashes_for_different_inputs() {
        let a = fnv1a_64(b"hello");
        let b = fnv1a_64(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn should_handle_empty_input() {
        let hash = fnv1a_64(b"");
        assert_eq!(hash, 0xcbf29ce484222325);
    }
}
