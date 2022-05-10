use std::{
    alloc::{handle_alloc_error, Layout},
    ptr::slice_from_raw_parts,
};

pub struct RingBuffer {
    buf: *mut u8,
    cap: usize,
    head: usize,
    tail: usize,
}

impl RingBuffer {
    pub const fn new() -> Self {
        RingBuffer {
            buf: std::ptr::null_mut(),
            cap: 0,
            head: 0,
            tail: 0,
        }
    }

    pub const fn len(&self) -> usize {
        let (x, y) = self.data_slice_lengths();
        x + y
    }

    pub const fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
    }

    pub const fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    pub fn reserve(&mut self, amount: usize) {
        if self.cap - self.len() > amount {
            return;
        }

        unsafe {
            self.reserve_amortized(amount);
        }
    }

    #[inline(never)]
    #[cold]
    unsafe fn reserve_amortized(&mut self, amount: usize) {
        debug_assert!(amount > 0);

        // SAFETY: is we were succesfully able to construct this layout when we allocated then it's also valid do so now
        let current_layout = Layout::array::<u8>(self.cap).unwrap_unchecked();

        let new_cap = usize::max(self.cap * 2, (self.cap + amount + 1).next_power_of_two());

        // Check that the capacity isn't bigger than isize::MAX, which is the max allowed by LLVM, or that
        // we are on a >= 64 bit system which will never allow that much memory to be allocated
        assert!(usize::BITS >= 64 || new_cap < isize::MAX as usize);

        let new_layout = Layout::array::<u8>(new_cap).unwrap();

        let new_buf = std::alloc::alloc(new_layout);

        if new_buf.is_null() {
            handle_alloc_error(new_layout);
        }

        if self.cap > 0 {
            let ((s1_ptr, s1_len), (s2_ptr, s2_len)) = self.data_slice_parts();

            new_buf.copy_from_nonoverlapping(s1_ptr, s1_len);
            new_buf.add(s1_len).copy_from_nonoverlapping(s2_ptr, s2_len);
            std::alloc::dealloc(self.buf, current_layout);

            self.tail = s1_len + s2_len;
            self.head = 0;
        }
        self.buf = new_buf;
        self.cap = new_cap;
    }

    pub fn extend(&mut self, data: &[u8]) {
        let len = data.len();
        let ptr = data.as_ptr();

        self.reserve(len);

        let ((f1_ptr, f1_len), (f2_ptr, f2_len)) = self.free_slice_parts();
        debug_assert!(f1_len + f2_len >= len, "{} + {} < {}", f1_len, f2_len, len);

        let in_f1 = usize::min(len, f1_len);
        let in_f2 = len - in_f1;

        debug_assert!(in_f1 + in_f2 == len);

        unsafe {
            if in_f1 > 0 {
                f1_ptr.copy_from_nonoverlapping(ptr, in_f1);
            }
            if in_f2 > 0 {
                f2_ptr.copy_from_nonoverlapping(ptr.add(in_f1), in_f2);
            }
        }
        self.tail = (self.tail + len) % self.cap;
    }

    pub fn drain(&mut self, amount: usize) {
        debug_assert!(amount <= self.len());
        let amount = usize::min(amount, self.len());
        self.head = (self.head + amount) % self.cap;
    }

    const fn data_slice_lengths(&self) -> (usize, usize) {
        let len_after_head;
        let len_to_tail;

        // TODO can we do this branchless?
        if self.tail >= self.head {
            len_after_head = self.tail - self.head;
            len_to_tail = 0;
        } else {
            len_after_head = self.cap - self.head;
            len_to_tail = self.tail;
        }
        (len_after_head, len_to_tail)
    }

    const fn data_slice_parts(&self) -> ((*const u8, usize), (*const u8, usize)) {
        let (len_after_head, len_to_tail) = self.data_slice_lengths();

        (
            (unsafe { self.buf.add(self.head) }, len_after_head),
            (self.buf, len_to_tail),
        )
    }
    pub const fn as_slices(&self) -> (&[u8], &[u8]) {
        let (s1, s2) = self.data_slice_parts();
        unsafe {
            let s1 = &*slice_from_raw_parts(s1.0, s1.1);
            let s2 = &*slice_from_raw_parts(s2.0, s2.1);
            (s1, s2)
        }
    }

    const fn free_slice_lengths(&self) -> (usize, usize) {
        let len_to_head;
        let len_after_tail;

        // TODO can we do this branchless?
        if self.tail < self.head {
            len_after_tail = self.head - self.tail;
            len_to_head = 0;
        } else {
            len_after_tail = self.cap - self.tail;
            len_to_head = self.head;
        }
        (len_to_head, len_after_tail)
    }

    const fn free_slice_parts(&self) -> ((*mut u8, usize), (*mut u8, usize)) {
        let (len_to_head, len_after_tail) = self.free_slice_lengths();

        (
            (unsafe { self.buf.add(self.tail) }, len_after_tail),
            (self.buf, len_to_head),
        )
    }

    #[allow(dead_code)]
    pub fn extend_from_within(&mut self, start: usize, len: usize) {
        if start + len > self.len() {
            ring_buffer_out_of_bounds();
        }

        self.reserve(len);
        unsafe { self.extend_from_within_unchecked(start, len) }
    }

    /// SAFETY:
    /// Needs start + len <= self.len()
    /// And more then len reserved space
    #[warn(unsafe_op_in_unsafe_fn)]
    pub unsafe fn extend_from_within_unchecked(&mut self, start: usize, len: usize) {
        debug_assert!(!self.buf.is_null());

        if self.head < self.tail {
            // continous data slice  |____HDDDDDDDT_____|
            let after_tail = usize::min(len, self.cap - self.tail);
            unsafe {
                self.buf
                    .add(self.tail)
                    .copy_from_nonoverlapping(self.buf.add(self.head + start), after_tail);
                if after_tail < len {
                    self.buf.copy_from_nonoverlapping(
                        self.buf.add(self.head + start + after_tail),
                        len - after_tail,
                    );
                }
            }
        } else {
            // continous free slice |DDDT_________HDDDD|
            if self.head + start > self.cap {
                let start = (self.head + start) % self.cap;
                unsafe {
                    self.buf
                        .add(self.tail)
                        .copy_from_nonoverlapping(self.buf.add(start), len)
                }
            } else {
                let after_head = usize::min(len, self.cap - self.head);
                unsafe {
                    self.buf
                        .add(self.tail)
                        .copy_from_nonoverlapping(self.buf.add(self.head + start), after_head);
                    if after_head < len {
                        self.buf
                            .add(self.tail + after_head)
                            .copy_from_nonoverlapping(self.buf, len - after_head);
                    }
                }
            }
        }

        self.tail = (self.tail + len) % self.cap;
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        if self.cap == 0 {
            return;
        }

        unsafe {
            std::alloc::dealloc(self.buf, Layout::array::<u8>(self.cap).unwrap_unchecked());
        }
    }
}

#[track_caller]
#[inline(never)]
#[cold]
const fn ring_buffer_out_of_bounds() {
    panic!("This is illegal!");
}

#[test]
fn smoke() {
    let mut rb = RingBuffer::new();

    rb.reserve(15);
    assert_eq!(16, rb.cap);

    rb.extend(b"0123456789");
    assert_eq!(rb.len(), 10);
    assert_eq!(rb.as_slices().0, b"0123456789");
    assert_eq!(rb.as_slices().1, b"");

    rb.drain(5);
    assert_eq!(rb.len(), 5);
    assert_eq!(rb.as_slices().0, b"56789");
    assert_eq!(rb.as_slices().1, b"");

    rb.extend_from_within(2, 3);
    assert_eq!(rb.len(), 8);
    assert_eq!(rb.as_slices().0, b"56789789");
    assert_eq!(rb.as_slices().1, b"");

    rb.extend_from_within(0, 3);
    assert_eq!(rb.len(), 11);
    assert_eq!(rb.as_slices().0, b"56789789567");
    assert_eq!(rb.as_slices().1, b"");

    rb.extend_from_within(0, 2);
    assert_eq!(rb.len(), 13);
    assert_eq!(rb.as_slices().0, b"56789789567");
    assert_eq!(rb.as_slices().1, b"56");

    rb.drain(11);
    assert_eq!(rb.len(), 2);
    assert_eq!(rb.as_slices().0, b"56");
    assert_eq!(rb.as_slices().1, b"");

    rb.extend(b"0123456789");
    assert_eq!(rb.len(), 12);
    assert_eq!(rb.as_slices().0, b"560123456789");
    assert_eq!(rb.as_slices().1, b"");

    rb.drain(11);
    assert_eq!(rb.len(), 1);
    assert_eq!(rb.as_slices().0, b"9");
    assert_eq!(rb.as_slices().1, b"");

    rb.extend(b"0123456789");
    assert_eq!(rb.len(), 11);
    assert_eq!(rb.as_slices().0, b"90123");
    assert_eq!(rb.as_slices().1, b"456789");
}
