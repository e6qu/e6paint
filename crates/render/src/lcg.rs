//! Deterministic per-element randomness for the sketch theme (multiplicative LCG,
//! Kaneko constants — visual scatter, not crypto).

/// Seedable LCG.
#[derive(Debug, Clone)]
pub struct Lcg(u64);

impl Lcg {
    pub fn from_seed(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    /// Uniform in `[0, 1)`.
    pub fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 40) as f64 / (1u64 << 24) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        let mut a = Lcg::from_seed(7);
        let mut b = Lcg::from_seed(7);
        for _ in 0..64 {
            let value = a.next_unit();
            assert_eq!(value, b.next_unit());
            assert!((0.0..1.0).contains(&value));
        }
        assert_ne!(Lcg::from_seed(7).next_unit(), Lcg::from_seed(8).next_unit());
    }
}
