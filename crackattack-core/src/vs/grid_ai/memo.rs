//! Memo of a search that depends only on the board and a few options: the answer is reused while
//! the key (board hash and the options that shape the search) is unchanged. Purely an
//! optimisation; a memo that always missed would give the same decisions.

pub(super) struct Memo<K, T> {
    key: Option<K>,
    value: Option<T>,
}

impl<K: PartialEq, T: Copy> Memo<K, T> {
    pub(super) fn new() -> Self {
        Self { key: None, value: None }
    }

    pub(super) fn get(&mut self, key: K, compute: impl FnOnce() -> Option<T>) -> Option<T> {
        if self.key.as_ref() != Some(&key) {
            self.value = compute();
            self.key = Some(key);
        }
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::Memo;

    #[test]
    fn computes_once_per_key_even_for_an_absent_answer() {
        let mut memo: Memo<u32, i32> = Memo::new();
        let mut calls = 0;
        for _ in 0..3 {
            assert_eq!(memo.get(1, || { calls += 1; None }), None);
        }
        assert_eq!(calls, 1);
        assert_eq!(memo.get(2, || { calls += 1; Some(7) }), Some(7));
        assert_eq!(memo.get(2, || { calls += 1; Some(8) }), Some(7));
        assert_eq!(calls, 2);
    }
}
