//! Technical indicators for backtesting.

/// Compute Simple Moving Average for a slice of values.
///
/// Returns a `Vec` of the same length as `values`.
/// The first `period - 1` elements are `None`; from index `period - 1`
/// onward the value is `Some(mean of the trailing `period` elements)`.
pub fn sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    if period == 0 {
        return vec![None; values.len()];
    }
    let mut result = Vec::with_capacity(values.len());
    let mut window_sum: f64 = 0.0;
    for (i, &v) in values.iter().enumerate() {
        window_sum += v;
        if i >= period {
            window_sum -= values[i - period];
        }
        if i + 1 >= period {
            result.push(Some(window_sum / period as f64));
        } else {
            result.push(None);
        }
    }
    result
}

/// Compute Relative Strength Index (RSI) using Wilder's smoothing.
///
/// Returns a `Vec` of the same length as `values`.
/// The first `period` elements are `None` (need `period + 1` price points
/// to compute `period` deltas). From index `period` onward the value is
/// `Some(rsi)` where RSI ∈ [0, 100].
///
/// Wilder's smoothing: `avg = (prev_avg * (period - 1) + current) / period`.
pub fn rsi(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = values.len();
    if period == 0 || n < period + 1 {
        return vec![None; n];
    }

    let mut result = Vec::with_capacity(n);
    // First `period` elements: not enough data.
    for _ in 0..period {
        result.push(None);
    }

    // Compute initial average gain/loss from the first `period` deltas.
    let mut avg_gain: f64 = 0.0;
    let mut avg_loss: f64 = 0.0;
    for i in 1..=period {
        let delta = values[i] - values[i - 1];
        if delta > 0.0 {
            avg_gain += delta;
        } else {
            avg_loss += -delta;
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    // RSI at index `period`.
    let rs = if avg_loss == 0.0 { 100.0 } else { avg_gain / avg_loss };
    let rsi_val = if avg_loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + rs) };
    result.push(Some(rsi_val));

    // Wilder's smoothing for the remaining bars.
    for i in (period + 1)..n {
        let delta = values[i] - values[i - 1];
        let gain = if delta > 0.0 { delta } else { 0.0 };
        let loss = if delta < 0.0 { -delta } else { 0.0 };

        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;

        let r = if avg_loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + avg_gain / avg_loss) };
        result.push(Some(r));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sma_basic() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = sma(&values, 3);
        assert_eq!(result.len(), 5);
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert_eq!(result[2], Some(2.0)); // (1+2+3)/3
        assert_eq!(result[3], Some(3.0)); // (2+3+4)/3
        assert_eq!(result[4], Some(4.0)); // (3+4+5)/3
    }

    #[test]
    fn test_sma_period_one() {
        let values = vec![10.0, 20.0, 30.0];
        let result = sma(&values, 1);
        assert_eq!(result, vec![Some(10.0), Some(20.0), Some(30.0)]);
    }

    #[test]
    fn test_sma_period_equals_len() {
        let values = vec![2.0, 4.0, 6.0];
        let result = sma(&values, 3);
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert_eq!(result[2], Some(4.0));
    }

    #[test]
    fn test_sma_period_exceeds_len() {
        let values = vec![1.0, 2.0];
        let result = sma(&values, 5);
        assert_eq!(result, vec![None, None]);
    }

    #[test]
    fn test_sma_empty() {
        let result = sma(&[], 3);
        assert!(result.is_empty());
    }

    #[test]
    fn test_sma_zero_period() {
        let result = sma(&[1.0, 2.0], 0);
        assert_eq!(result, vec![None, None]);
    }

    // ---- RSI tests ----

    #[test]
    fn test_rsi_basic() {
        // Ascending prices → RSI should approach 100.
        let values: Vec<f64> = (0..20).map(|i| 100.0 + i as f64).collect();
        let result = rsi(&values, 14);
        assert_eq!(result.len(), 20);
        // First 14 should be None.
        for v in &result[..14] {
            assert_eq!(*v, None);
        }
        // After period, all gains → RSI should be 100.
        for v in &result[14..] {
            let r = v.unwrap();
            assert!((r - 100.0).abs() < 0.01, "Expected ~100 for all-up, got {r}");
        }
    }

    #[test]
    fn test_rsi_descending() {
        // Descending prices → RSI should be 0.
        let values: Vec<f64> = (0..20).map(|i| 100.0 - i as f64).collect();
        let result = rsi(&values, 14);
        for v in &result[14..] {
            let r = v.unwrap();
            assert!(r.abs() < 0.01, "Expected ~0 for all-down, got {r}");
        }
    }

    #[test]
    fn test_rsi_insufficient_data() {
        let values = vec![1.0, 2.0, 3.0];
        let result = rsi(&values, 5);
        assert_eq!(result, vec![None, None, None]);
    }

    #[test]
    fn test_rsi_zero_period() {
        let result = rsi(&[1.0, 2.0, 3.0], 0);
        assert_eq!(result, vec![None, None, None]);
    }
}
