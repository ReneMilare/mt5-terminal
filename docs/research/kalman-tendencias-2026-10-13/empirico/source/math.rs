//! Numeric routines ported from `StochasticMath.mqh`, `ContextMath.mqh` and `RegimeMath.mqh`.
//! `f64::NAN` stands for MQL's `EMPTY_VALUE`.

/// Sample variance of `values` (all finite, at least 3, not ~0), as `StochMoments`.
pub fn variance(values: &[f64]) -> Option<f64> {
    let (mut n, mut mean, mut m2) = (0.0, 0.0, 0.0);
    for &x in values {
        if !x.is_finite() {
            return None;
        }
        n += 1.0;
        let d = x - mean;
        mean += d / n;
        m2 += d * (x - mean);
    }
    let var = if n > 1.0 { (m2 / (n - 1.0)).max(0.0) } else { 0.0 };
    (n >= 3.0 && var > 1e-18).then_some(var)
}

pub fn log_return(before: f64, after: f64) -> f64 {
    if before > 0.0 && after > 0.0 { (after / before).ln() } else { f64::NAN }
}

/// Local linear trend (level + slope) Kalman filter on log price.
#[derive(Clone, Copy, Debug, Default)]
pub struct Kalman {
    pub level: f64,
    pub slope: f64,
    pub p00: f64,
    pub p01: f64,
    pub p11: f64,
    /// Last update's innovation (observed − predicted) and its variance.
    pub innovation: f64,
    pub innovation_var: f64,
}

impl Kalman {
    pub fn new(log_price: f64, variance: f64) -> Self {
        Self { level: log_price, slope: 0.0, p00: variance, p01: 0.0, p11: variance, ..Self::default() }
    }

    pub fn update(&mut self, observed: f64, q_level: f64, q_slope: f64, r: f64) -> bool {
        if q_level <= 0.0 || q_slope <= 0.0 || r <= 0.0 || !observed.is_finite() {
            return false;
        }
        let a = self.p00 + 2.0 * self.p01 + self.p11 + q_level;
        let b = self.p01 + self.p11;
        let c = self.p11 + q_slope;
        let innovation = a + r;
        if innovation <= 0.0 || !innovation.is_finite() {
            return false;
        }
        let (k0, k1) = (a / innovation, b / innovation);
        let predicted = self.level + self.slope;
        let e = observed - predicted;
        (self.innovation, self.innovation_var) = (e, innovation);
        self.level = predicted + k0 * e;
        self.slope += k1 * e;
        // Joseph form keeps the covariance symmetric and positive semidefinite
        self.p00 = (1.0 - k0) * (1.0 - k0) * a + k0 * k0 * r;
        self.p01 = (1.0 - k0) * (b - k1 * a) + k0 * k1 * r;
        self.p11 = c - 2.0 * k1 * b + k1 * k1 * a + k1 * k1 * r;
        self.level.is_finite() && self.slope.is_finite() && self.p00 > 0.0 && self.p11 > 0.0 && self.p00 * self.p11 - self.p01 * self.p01 >= -1e-24
    }

}

/// Weighted Welford accumulator (stable at index price levels).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Weighted {
    pub total: f64,
    pub mean: f64,
    pub m2: f64,
}

impl Weighted {
    pub fn add(&mut self, price: f64, weight: f64) {
        if weight <= 0.0 {
            return;
        }
        let next = self.total + weight;
        let delta = price - self.mean;
        self.mean += delta * weight / next;
        self.m2 += weight * delta * (price - self.mean);
        self.total = next;
    }
}

/// Value area around the POC: (poc, lo, hi) bin indices. Lowest bin wins POC ties.
pub fn value_area(bins: &[f64], fraction: f64) -> Option<(usize, usize, usize)> {
    let count = bins.len();
    if count == 0 || fraction <= 0.0 || fraction > 1.0 {
        return None;
    }
    let (mut total, mut peak, mut poc) = (0.0, 0.0, 0usize);
    for (i, &b) in bins.iter().enumerate() {
        total += b;
        if b > peak {
            peak = b;
            poc = i;
        }
    }
    if total <= 0.0 {
        return None;
    }
    let (mut lo, mut hi, mut covered) = (poc, poc, bins[poc]);
    while covered < total * fraction && (lo > 0 || hi < count - 1) {
        let left = if lo > 0 { bins[lo - 1] } else { -1.0 };
        let right = if hi < count - 1 { bins[hi + 1] } else { -1.0 };
        if left >= right && lo > 0 {
            lo -= 1;
            covered += bins[lo];
        }
        if right >= left && hi < count - 1 {
            hi += 1;
            covered += bins[hi];
        }
    }
    Some((poc, lo, hi))
}

/// Lo–MacKinlay variance ratio VR(q) and its heteroskedasticity-robust z over `r`.
pub fn variance_ratio(r: &[f64], q: usize) -> Option<(f64, f64)> {
    let n = r.len();
    if q < 2 || n < 4 * q {
        return None;
    }
    let mut mu = 0.0;
    for &x in r {
        if !x.is_finite() {
            return None;
        }
        mu += x;
    }
    mu /= n as f64;
    let ss: f64 = r.iter().map(|x| (x - mu) * (x - mu)).sum();
    if ss <= 1e-24 {
        return None;
    }
    let var_a = ss / (n as f64 - 1.0);
    let qf = q as f64;
    let mut sum: f64 = r[..q].iter().sum();
    let mut sc = (sum - qf * mu) * (sum - qf * mu);
    for k in q..n {
        sum += r[k] - r[k - q];
        sc += (sum - qf * mu) * (sum - qf * mu);
    }
    let m = qf * (n as f64 - qf + 1.0) * (1.0 - qf / n as f64);
    let vr = (sc / m) / var_a;
    let mut theta = 0.0;
    for j in 1..q {
        let num: f64 = (j..n).map(|k| (r[k] - mu) * (r[k] - mu) * (r[k - j] - mu) * (r[k - j] - mu)).sum();
        let w = 2.0 * (q - j) as f64 / qf;
        theta += w * w * n as f64 * num / (ss * ss);
    }
    if theta <= 0.0 {
        return None;
    }
    let z = (n as f64).sqrt() * (vr - 1.0) / theta.sqrt();
    (vr.is_finite() && z.is_finite()).then_some((vr, z))
}

/// Percentile (0–100) of `x` among `reference`; ties count half.
pub fn percentile(x: f64, reference: &[f64]) -> f64 {
    if reference.is_empty() {
        return f64::NAN;
    }
    let below: f64 = reference.iter().map(|&v| if v < x { 1.0 } else if v == x { 0.5 } else { 0.0 }).sum();
    100.0 * below / reference.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_area_grows_from_poc() {
        // 70% of 15 = 10.5: POC 8, then the larger neighbour (3) reaches 11
        let bins = [1.0, 2.0, 8.0, 3.0, 1.0, 0.0];
        assert_eq!(value_area(&bins, 0.7), Some((2, 2, 3)));
        // equal neighbours both join
        assert_eq!(value_area(&[1.0, 2.0, 8.0, 2.0, 1.0], 0.8), Some((2, 1, 3)));
        assert_eq!(value_area(&[0.0, 0.0], 0.7), None);
    }

    #[test]
    fn weighted_matches_direct() {
        let mut w = Weighted::default();
        for (p, v) in [(10.0, 1.0), (12.0, 3.0), (11.0, 0.0), (9.0, 2.0)] {
            w.add(p, v);
        }
        let mean = (10.0 + 36.0 + 18.0) / 6.0;
        assert!((w.mean - mean).abs() < 1e-12);
        let var = (1.0 * (10.0f64 - mean).powi(2) + 3.0 * (12.0f64 - mean).powi(2) + 2.0 * (9.0f64 - mean).powi(2)) / 6.0;
        assert!((w.m2 / w.total - var).abs() < 1e-12);
    }

    #[test]
    fn variance_ratio_signs() {
        // alternating returns revert, a steady drift with noise persists
        let revert: Vec<f64> = (0..120).map(|i| if i % 2 == 0 { 0.001 } else { -0.001 } + (i as f64 * 0.37).sin() * 1e-4).collect();
        let (vr, z) = variance_ratio(&revert, 6).unwrap();
        assert!(vr < 1.0 && z < 0.0);
        let persist: Vec<f64> = (0..120).map(|i| ((i / 12) % 2) as f64 * 0.002 - 0.001 + (i as f64 * 0.71).sin() * 1e-4).collect();
        let (vr, z) = variance_ratio(&persist, 6).unwrap();
        assert!(vr > 1.0 && z > 0.0);
    }

    #[test]
    fn percentile_ties() {
        assert_eq!(percentile(2.0, &[1.0, 2.0, 3.0, 4.0]), 37.5);
    }
}
