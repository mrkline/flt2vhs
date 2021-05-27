use bit_vec::BitVec;

pub struct IdSet {
    used_ids: BitVec,
    next_unused: usize,
}

impl IdSet {
    pub fn from_ids<I: Iterator<Item = i32> + Clone>(used_ids: I) -> Self {
        let max_id = used_ids.clone().max().unwrap() as usize;
        let mut bitmap = BitVec::from_elem(max_id + 1, false);
        for id in used_ids {
            assert!(id >= 0);
            bitmap.set(id as usize, true);
        }
        Self {
            used_ids: bitmap,
            next_unused: 0,
        }
    }

    pub fn next_unused(&mut self) -> i32 {
        while self.next_unused < self.used_ids.len() {
            if !self.used_ids.get(self.next_unused).unwrap() {
                break;
            }
            self.next_unused += 1;
        }
        // After we go through pre-existing IDs, new ones are unused.
        let unused = self.next_unused;
        self.next_unused += 1;

        assert!(unused <= i32::MAX as usize);
        unused as i32
    }

    #[inline]
    pub fn contains(&self, id: i32) -> bool {
        assert!(id >= 0);
        self.used_ids.get(id as usize).unwrap_or(false)
    }
}
