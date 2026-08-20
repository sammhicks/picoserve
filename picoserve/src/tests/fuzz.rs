use std::{boxed::Box, eprintln, string::String};

use rand::{Rng, RngExt};

pub struct FragmentedWriter<W> {
    rng: rand_pcg::Pcg64,
    writer: W,
}

impl<W: crate::io::ErrorType> crate::io::ErrorType for FragmentedWriter<W> {
    type Error = W::Error;
}

impl<W: crate::io::BaseWrite> crate::io::BaseWrite for FragmentedWriter<W> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let write_size = self.rng.random_range(1..=buf.len());

        self.writer.write(&buf[..write_size]).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.writer.flush().await
    }
}

impl<W: crate::io::Write> crate::io::Write for FragmentedWriter<W> {
    fn write_with<F: FnOnce(&mut crate::mem::BorrowedBuffer<'_>) -> R, R>(
        &mut self,
        f: F,
    ) -> impl core::future::Future<Output = Result<R, Self::Error>> {
        self.writer.write_with(|buffer| {
            let buffer_capacity = buffer.capacity();

            let mut cursor = buffer.unfilled();

            let mut fragment_buffer = crate::mem::BorrowedBuffer::new(
                &mut cursor.as_mut_slice()[..self.rng.random_range(1..=buffer_capacity)],
            );

            let output = f(&mut fragment_buffer);

            let fragment_length = fragment_buffer.len();

            buffer.unfilled().advance(fragment_length);

            output
        })
    }
}

pub trait TestValue<Parameter> {
    fn generate(test_data: &mut TestData, parameter: Parameter) -> Self;
}

macro_rules! int_test_value {
    ($($t:ty),* $(,)?) => {
        $(
            impl TestValue<()> for $t {
                fn generate(test_data: &mut TestData, (): ()) -> Self {
                    test_data.rng.random()
                }
            }

            impl TestValue<core::ops::Range<Self>> for $t {
                fn generate(test_data: &mut TestData, range: core::ops::Range<Self>) -> Self {
                    test_data.rng.random_range(range)
                }
            }

            impl TestValue<core::ops::RangeInclusive<Self>> for $t {
                fn generate(test_data: &mut TestData, range: core::ops::RangeInclusive<Self>) -> Self {
                    test_data.rng.random_range(range)
                }
            }
        )*
    };
}

int_test_value!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

impl TestValue<core::ops::Range<Self>> for usize {
    fn generate(test_data: &mut TestData, range: core::ops::Range<Self>) -> Self {
        test_data.rng.random_range(range)
    }
}

impl TestValue<core::ops::RangeInclusive<Self>> for usize {
    fn generate(test_data: &mut TestData, range: core::ops::RangeInclusive<Self>) -> Self {
        test_data.rng.random_range(range)
    }
}

impl TestValue<()> for bool {
    fn generate(test_data: &mut TestData, (): ()) -> Self {
        test_data.rng.random()
    }
}

impl TestValue<core::ops::Range<usize>> for String {
    fn generate(test_data: &mut TestData, length_range: core::ops::Range<usize>) -> Self {
        use rand::distr::Distribution;

        let length = test_data.rng.random_range(length_range);

        let mut s = Self::with_capacity(length);

        loop {
            let c = rand::distr::Alphanumeric.sample(&mut test_data.rng) as char;

            if s.len() + c.encode_utf8(&mut [0; 4]).len() > length {
                return s;
            }

            s.push(c);
        }
    }
}

impl TestValue<usize> for std::vec::Vec<u8> {
    fn generate(test_data: &mut TestData, length: usize) -> Self {
        let mut blob = std::vec![0; length];

        test_data.rng.fill_bytes(&mut blob);

        blob
    }
}

impl TestValue<core::ops::Range<usize>> for std::vec::Vec<u8> {
    fn generate(test_data: &mut TestData, length_range: core::ops::Range<usize>) -> Self {
        let mut blob = std::vec![0; test_data.generate_value_with_parameter(length_range)];

        test_data.rng.fill_bytes(&mut blob);

        blob
    }
}

impl TestValue<core::ops::RangeInclusive<usize>> for std::vec::Vec<u8> {
    fn generate(test_data: &mut TestData, length_range: core::ops::RangeInclusive<usize>) -> Self {
        let mut blob = std::vec![0; test_data.generate_value_with_parameter(length_range)];

        test_data.rng.fill_bytes(&mut blob);

        blob
    }
}

pub struct TestData {
    rng: rand_pcg::Pcg64,
}

impl TestData {
    pub fn choose_value<'a, T>(&mut self, values: &'a [T]) -> &'a T {
        use rand::seq::IndexedRandom;

        values.choose(&mut self.rng).unwrap()
    }

    pub fn derived(&mut self) -> TestData {
        TestData {
            rng: rand_pcg::Pcg64::new(self.generate_value(), self.generate_value()),
        }
    }

    pub fn generate_replayable(&mut self) -> ReplayableTestData {
        ReplayableTestData {
            test_data: self.derived(),
        }
    }

    pub fn generate_value_with_parameter<T: TestValue<P>, P>(&mut self, parameter: P) -> T {
        T::generate(self, parameter)
    }

    pub fn generate_value<T: TestValue<()>>(&mut self) -> T {
        self.generate_value_with_parameter(())
    }

    pub fn generate_string(&mut self, length_range: core::ops::Range<usize>) -> String {
        self.generate_value_with_parameter(length_range)
    }

    pub fn generate_blob<R>(&mut self, length_range: R) -> std::vec::Vec<u8>
    where
        std::vec::Vec<u8>: TestValue<R>,
    {
        self.generate_value_with_parameter(length_range)
    }

    pub fn generate_fragmented_writer<W>(&mut self, writer: W) -> FragmentedWriter<W> {
        FragmentedWriter {
            rng: rand_pcg::Pcg64::new(self.generate_value(), self.generate_value()),
            writer,
        }
    }
}

pub struct ReplayableTestData {
    test_data: TestData,
}

impl ReplayableTestData {
    pub fn start(&self) -> TestData {
        let Self {
            test_data: TestData { rng },
        } = self;

        TestData { rng: rng.clone() }
    }
}

struct TestInfo {
    test_data: TestData,
}

static TEST_RNG_SEED: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("TEST_RNG_SEED")
        .ok()
        .unwrap_or(std::format!("{:0x}", rand::random::<u64>()))
});

fn setup(test_name: impl AsRef<str>) -> TestInfo {
    use sha2::Digest;

    let rng = {
        let mut hasher = sha2::Sha256::new();

        hasher.update(test_name.as_ref());
        hasher.update(TEST_RNG_SEED.as_bytes());

        let &[state, stream] = hasher.finalize().0.as_chunks().0.as_array().unwrap();

        rand_pcg::Pcg64::new(u128::from_ne_bytes(state), u128::from_ne_bytes(stream))
    };

    TestInfo {
        test_data: TestData { rng },
    }
}

pub fn run_sync(test_name: impl AsRef<str>, test: impl Fn(&mut TestData)) {
    let TestInfo { mut test_data } = setup(test_name);

    std::panic::set_hook(Box::new(move |panic_info| {
        eprintln!("TEST_RNG_SEED={}", TEST_RNG_SEED.as_str());
        eprintln!("{panic_info}");
    }));

    for _ in 0..100 {
        test(&mut test_data)
    }

    _ = std::panic::take_hook();
}

pub async fn run_async(test_name: impl AsRef<str>, test: impl AsyncFn(&mut TestData)) {
    let TestInfo { mut test_data } = setup(test_name);

    std::panic::set_hook(Box::new(move |panic_info| {
        eprintln!("TEST_RNG_SEED={}", TEST_RNG_SEED.as_str());
        eprintln!("{panic_info}");
    }));

    for _ in 0..100 {
        test(&mut test_data).await
    }

    _ = std::panic::take_hook();
}
