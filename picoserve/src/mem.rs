/// A borrowed byte buffer which is incrementally filled.
pub struct BorrowedBuffer<'data> {
    /// The buffer's underlying data.
    buffer: &'data mut [u8],
    /// The length of `self.buf` which is known to be filled.
    ///
    /// Invariant: `filled <= buffer.len()`
    filled: usize,
}

impl<'data> BorrowedBuffer<'data> {
    /// Create an unfilled buffer using `buffer` as backing storage.
    pub fn new(buffer: &'data mut [u8]) -> Self {
        Self { buffer, filled: 0 }
    }

    /// Returns the total capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// Returns the length of the filled part of the buffer.
    pub fn len(&self) -> usize {
        self.filled
    }

    /// Returns true if the buffer length is zero.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a shared reference to the filled portion of the buffer.
    pub fn filled(&self) -> &[u8] {
        debug_assert!(self.filled <= self.buffer.len());

        // Safety: We only slice the filled part of the buffer, which is always valid
        #[allow(unsafe_code, reason = "See safety comment")]
        unsafe {
            self.buffer.get_unchecked(..self.filled)
        }
    }

    /// Returns a mutable reference to the filled portion of the buffer.
    pub fn filled_mut(&mut self) -> &mut [u8] {
        debug_assert!(self.filled <= self.buffer.len());

        // Safety: We only slice the filled part of the buffer, which is always valid
        #[allow(unsafe_code, reason = "See safety comment")]
        unsafe {
            self.buffer.get_unchecked_mut(..self.filled)
        }
    }

    /// Returns a cursor over the unfilled part of the buffer.
    pub fn unfilled(&mut self) -> BorrowedCursor<'_> {
        let Self { buffer, filled } = self;

        BorrowedCursor { buffer, filled }
    }
}

/// The error produced by [`BorrowedCursor::try_append`]
#[derive(Debug)]
pub struct FailedToAppendError<'a> {
    /// The portion of data that was appended to the [`BorrowedCursor`].
    pub written_data: &'a [u8],
    /// The portion of data that could not be appended to the [`BorrowedCursor`].
    pub unwritten_data: &'a [u8],
}

#[derive(Clone, Copy)]
pub struct BorrowedCursorPosition(usize);

impl core::ops::Sub for BorrowedCursorPosition {
    type Output = usize;

    fn sub(self, rhs: Self) -> Self::Output {
        self.0 - rhs.0
    }
}

/// A writeable view of the unfilled portion of a [`BorrowedBuffer`].
pub struct BorrowedCursor<'a> {
    /// The buffer's underlying data.
    buffer: &'a mut [u8],
    /// The number of bytes in the buffer known to be filled.
    filled: &'a mut usize,
}

impl BorrowedCursor<'_> {
    /// Reborrows this cursor by cloning it with a smaller lifetime.
    pub fn reborrow(&mut self) -> BorrowedCursor<'_> {
        let Self { buffer, filled } = self;

        BorrowedCursor { buffer, filled }
    }

    /// Returns the available space in the cursor.
    pub fn remaining_capacity(&self) -> usize {
        BorrowedCursorPosition(self.buffer.len()) - self.position()
    }

    /// Returns the current [`position`](BorrowedCursorPosition) of the cursor, which implements [`Sub`](core::ops::Sub).
    pub fn position(&self) -> BorrowedCursorPosition {
        BorrowedCursorPosition(*self.filled)
    }

    fn filled_count_and_unfilled_slice(&mut self) -> (&mut usize, &mut [u8]) {
        debug_assert!(*self.filled <= self.buffer.len());

        // Safety: always in bounds
        #[allow(unsafe_code, reason = "See safety comment")]
        let unfilled = unsafe { self.buffer.get_unchecked_mut(*self.filled..) };

        (self.filled, unfilled)
    }

    /// Returns a mutable reference to the whole cursor, i.e. the unfilled portion of the [`BorrowedBuffer`] this cursor was created from.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.filled_count_and_unfilled_slice().1
    }

    /// Advances the cursor by asserting that n elements have been filled.
    /// The cursor won't be advanced off the end of the buffer.
    pub fn advance(&mut self, n: usize) {
        *self.filled = self.filled.saturating_add(n).min(self.buffer.len());
    }

    /// Try to append `data` to the buffer.
    /// If there's not enough space in the buffer, as much data as possible is appended,
    /// and the returned [`FailedToAppendError`] indicates how much data was appended.
    pub fn try_append<'data>(
        &mut self,
        data: &'data [u8],
    ) -> Result<(), FailedToAppendError<'data>> {
        let (filled, buffer) = self.filled_count_and_unfilled_slice();

        let result = match data
            .split_at_checked(buffer.len())
            .filter(|(_written_data, unwritten_data)| !unwritten_data.is_empty())
        {
            Some((written_data, unwritten_data)) => {
                buffer.copy_from_slice(written_data);

                *filled += buffer.len();

                Err(FailedToAppendError {
                    written_data,
                    unwritten_data,
                })
            }
            None => {
                buffer[..data.len()].copy_from_slice(data);

                *filled += data.len();

                Ok(())
            }
        };

        debug_assert!(*self.filled <= self.buffer.len());

        result
    }

    /// Run the given closure with a [`BorrowedBuffer`] containing the unfilled part of the cursor.
    ///
    /// # Panics
    ///
    /// Panics if the `BorrowedBuf` given to the closure is replaced by another one.
    pub fn with_unfilled_buf<T>(&mut self, f: impl FnOnce(&mut BorrowedBuffer<'_>) -> T) -> T {
        let mut buffer = BorrowedBuffer::new(self.as_mut_slice());
        let previous_buffer_pointer = buffer.buffer.as_ptr();
        let output = f(&mut buffer);

        assert_eq!(previous_buffer_pointer, buffer.buffer.as_ptr());

        *self.filled += buffer.filled;

        output
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn zero_length_buffer() {
        crate::tests::fuzz::run_sync("zero_length_buffer", |test_data| {
            let mut buffer = BorrowedBuffer::new(&mut []);

            let data = test_data.generate_blob(1..=128);

            let FailedToAppendError {
                written_data,
                unwritten_data,
            } = buffer.unfilled().try_append(&data).unwrap_err();

            assert!(written_data.is_empty());
            assert_eq!(unwritten_data, data);
        });
    }

    #[test]
    fn exact_fit_try_append() {
        crate::tests::fuzz::run_sync("exact_fit_try_append", |test_data| {
            let mut buffer = test_data.generate_blob(1..=128);

            for data_split_point in 0..=buffer.len() {
                let data = test_data.generate_blob(buffer.len());
                let (first_data, second_data) = data.split_at(data_split_point);

                let mut buffer = BorrowedBuffer::new(&mut buffer);

                buffer.unfilled().try_append(first_data).unwrap();
                buffer.unfilled().try_append(second_data).unwrap();

                assert_eq!(buffer.filled(), data);
            }
        });
    }

    #[test]
    fn one_byte_too_big_try_append() {
        crate::tests::fuzz::run_sync("one_byte_too_big_try_append", |test_data| {
            let mut buffer = test_data.generate_blob(1..=128);

            for data_split_point in 0..=buffer.len() {
                let data = test_data.generate_blob(buffer.len() + 1);
                let (first_data, second_data) = data.split_at(data_split_point);

                let mut buffer = BorrowedBuffer::new(&mut buffer);

                buffer.unfilled().try_append(first_data).unwrap();

                let FailedToAppendError {
                    written_data,
                    unwritten_data,
                } = buffer.unfilled().try_append(second_data).unwrap_err();

                assert_eq!(written_data, &data[data_split_point..(data.len() - 1)]);
                assert_eq!(unwritten_data, core::slice::from_ref(data.last().unwrap()));
            }
        });
    }

    #[test]
    fn advance() {
        crate::tests::fuzz::run_sync("advance", |test_data| {
            let mut buffer = test_data.generate_blob(1..=128);

            for advance_by in 0..=buffer.len() {
                let mut buffer = BorrowedBuffer::new(&mut buffer);

                buffer.unfilled().advance(advance_by);

                assert_eq!(buffer.filled().len(), advance_by);
            }
        });
    }

    #[test]
    fn advance_max() {
        crate::tests::fuzz::run_sync("advance_max", |test_data| {
            let mut buffer = test_data.generate_blob(1..=128);
            let mut buffer = BorrowedBuffer::new(&mut buffer);

            buffer.unfilled().advance(usize::MAX);

            assert_eq!(buffer.len(), buffer.capacity());
        });
    }

    #[test]
    fn with_unfilled_buf() {
        crate::tests::fuzz::run_sync("with_unfilled_buf", |test_data| {
            let mut buffer = test_data.generate_blob(1..=128);

            for data_split_point in 0..=buffer.len() {
                let data = test_data.generate_blob(buffer.len());
                let (first_data, remaining_data) = data.split_at(data_split_point);
                for data_split_point in 0..=remaining_data.len() {
                    let (second_data, third_data) = remaining_data.split_at(data_split_point);

                    let mut buffer = BorrowedBuffer::new(&mut buffer);

                    buffer.unfilled().try_append(first_data).unwrap();

                    buffer.unfilled().with_unfilled_buf(|buffer| {
                        buffer.unfilled().try_append(second_data).unwrap();
                        buffer.unfilled().with_unfilled_buf(|buffer| {
                            buffer.unfilled().try_append(third_data).unwrap();
                        });
                    });

                    assert_eq!(buffer.filled(), data);
                }
            }
        });
    }

    #[test]
    fn buffer_invariants() {
        crate::tests::fuzz::run_sync("buffer_invariants", |test_data| {
            let mut buffer = test_data.generate_blob(1..=128);
            let mut buffer = BorrowedBuffer::new(&mut buffer);

            let start_position = buffer.unfilled().position();

            assert_eq!(start_position.0, 0);

            for _ in 1..=test_data.generate_value_with_parameter(1..=100) {
                if test_data.generate_value() {
                    _ = buffer
                        .unfilled()
                        .try_append(&test_data.generate_blob(0..=10));
                } else {
                    buffer
                        .unfilled()
                        .advance(test_data.generate_value_with_parameter(0..=10));
                }

                assert!(buffer.len() <= buffer.capacity());

                let cursor = buffer.unfilled();
                assert_eq!(
                    (cursor.position() - start_position) + cursor.remaining_capacity(),
                    buffer.capacity(),
                );
            }
        });
    }
}
