/// A horizontal value slider ("scrubber") for a run parameter.
#[derive(Debug, Clone)]
pub struct Scrubber {
    pub label: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub precision: usize,
}

impl Scrubber {
    pub fn new(label: &str, value: f64, min: f64, max: f64, step: f64, precision: usize) -> Self {
        Self {
            label: label.to_string(),
            value,
            min,
            max,
            step,
            precision,
        }
    }

    /// Fraction filled, in `0.0..=1.0`.
    pub fn ratio(&self) -> f64 {
        if self.max <= self.min {
            return 0.0;
        }
        ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    pub fn inc(&mut self, mult: f64) {
        self.value = (self.value + self.step * mult).min(self.max);
        self.round();
    }

    pub fn dec(&mut self, mult: f64) {
        self.value = (self.value - self.step * mult).max(self.min);
        self.round();
    }

    /// Set the value from a `0.0..=1.0` fraction (used for mouse scrubbing).
    pub fn set_ratio(&mut self, r: f64) {
        let r = r.clamp(0.0, 1.0);
        self.value = self.min + r * (self.max - self.min);
        self.round();
    }

    fn round(&mut self) {
        let factor = 10f64.powi(self.precision as i32);
        self.value = (self.value * factor).round() / factor;
    }

    pub fn value_string(&self) -> String {
        format!("{:.*}", self.precision, self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::Scrubber;

    #[test]
    fn ratio_at_midpoint() {
        let s = Scrubber::new("Temperature", 1.0, 0.0, 2.0, 0.05, 2);
        assert!((s.ratio() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn inc_respects_max() {
        let mut s = Scrubber::new("Temperature", 1.95, 0.0, 2.0, 0.05, 2);
        s.inc(1.0);
        assert_eq!(s.value, 2.0);
    }

    #[test]
    fn dec_respects_min() {
        let mut s = Scrubber::new("Temperature", 0.05, 0.0, 2.0, 0.05, 2);
        s.dec(1.0);
        assert_eq!(s.value, 0.0);
    }

    #[test]
    fn set_ratio_sets_value() {
        let mut s = Scrubber::new("Top P", 0.0, 0.0, 1.0, 0.05, 2);
        s.set_ratio(0.75);
        assert_eq!(s.value, 0.75);
    }
}
