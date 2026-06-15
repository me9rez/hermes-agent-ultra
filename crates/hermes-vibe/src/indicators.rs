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
}
