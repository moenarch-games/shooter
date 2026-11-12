//! The bots' deterministic random stream (SplitMix64): one state word, a fixed increment and an
//! integer finaliser. Every draw advances the state by exactly one step.

const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Resumes a stream from its state.
    #[must_use]
    pub const fn from_state(state: u64) -> Self {
        Self { state }
    }

    #[must_use]
    pub const fn state(self) -> u64 {
        self.state
    }

    pub const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        mix(self.state)
    }

    /// Uniform-ish in `0..bound` by multiply-shift on the high 32 bits: exactly one step per
    /// draw, bias below `bound / 2^32`. `bound = 0` yields 0.
    pub const fn below(&mut self, bound: u32) -> u32 {
        (((self.next_u64() >> 32) * bound as u64) >> 32) as u32
    }
}

/// SplitMix64 finaliser.
const fn mix(value: u64) -> u64 {
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_is_pinned_and_resumable() {
        // Reference SplitMix64 output for state 0.
        let mut rng = Rng::from_state(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        let mut resumed = Rng::from_state(rng.state());
        assert_eq!(resumed.next_u64(), rng.next_u64());
        assert_eq!(rng.below(0), 0);
    }
}
