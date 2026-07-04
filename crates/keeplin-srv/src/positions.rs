const MIN_CHAR: u8 = b' ';
const MAX_CHAR: u8 = b'~';

/// Generate a lexicographic string strictly between `prev` and `next`.
/// `None` means "before the first" or "after the last" respectively.
pub fn between(prev: Option<&str>, next: Option<&str>) -> String {
    let prev_bytes = prev.map(|s| s.as_bytes()).unwrap_or(&[]);
    let next_bytes = next.map(|s| s.as_bytes()).unwrap_or(&[]);

    let mut out = Vec::new();
    let mut i = 0;

    loop {
        let p = prev_bytes.get(i).copied().unwrap_or(MIN_CHAR - 1);
        let n = next_bytes.get(i).copied().unwrap_or(MAX_CHAR + 1);

        if n - p > 1 {
            out.push((p + n) / 2);
            return String::from_utf8(out).expect("valid ASCII");
        }

        // No room at this digit; copy prev's byte and continue to the next digit.
        out.push(p);
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_first_and_last() {
        let a = between(None, None);
        let b = between(Some(&a), None);
        let c = between(Some(&a), Some(&b));
        assert!(a < b);
        assert!(a < c);
        assert!(c < b);
    }

    #[test]
    fn between_many_inserts() {
        let mut pos = between(None, None);
        for _ in 0..100 {
            pos = between(Some(&pos), None);
        }
        // It should still be valid ASCII and monotonic.
        assert!(!pos.is_empty());
        assert!(pos.as_bytes()[0] >= MIN_CHAR && pos.as_bytes()[0] <= MAX_CHAR);
    }
}
