use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fixed-capacity, lock-free single-producer / single-consumer ring buffer.
///
/// One thread calls `push()`, another calls `pop()`. Thread-safety is
/// guaranteed by atomic head/tail indices without any mutex. `N` must be a
/// power of two (asserted at construction).
pub struct RingBuffer<T, const N: usize> {
    data: [UnsafeCell<MaybeUninit<T>>; N],
    head: AtomicUsize, // writer advances head
    tail: AtomicUsize, // reader advances tail
}

// SAFETY: single-producer, single-consumer — only one thread writes, one reads.
unsafe impl<T: Send, const N: usize> Send for RingBuffer<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for RingBuffer<T, N> {}

impl<T, const N: usize> RingBuffer<T, N> {
    pub fn new() -> Self {
        assert!(N.is_power_of_two(), "RingBuffer size must be a power of two");
        // SAFETY: MaybeUninit array initialised to uninit — that is its purpose.
        let data = unsafe {
            MaybeUninit::<[UnsafeCell<MaybeUninit<T>>; N]>::uninit().assume_init()
        };
        Self { data, head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }

    /// Push an item. Returns `Err(item)` if the buffer is full.
    pub fn push(&self, item: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & (N - 1);
        if next == self.tail.load(Ordering::Acquire) {
            return Err(item); // full
        }
        // SAFETY: only the producer writes to `head` slot.
        unsafe { (*self.data[head].get()).write(item) };
        self.head.store(next, Ordering::Release);
        Ok(())
    }

    /// Pop an item. Returns `None` if the buffer is empty.
    pub fn pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None; // empty
        }
        // SAFETY: only the consumer reads from `tail` slot, after `head` is visible.
        let item = unsafe { (*self.data[tail].get()).assume_init_read() };
        self.tail.store((tail + 1) & (N - 1), Ordering::Release);
        Some(item)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tail.load(Ordering::Acquire) == self.head.load(Ordering::Acquire)
    }
}

impl<T, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> Self { Self::new() }
}
