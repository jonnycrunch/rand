// Copyright 2018 Developers of the Rand project.
// Copyright 2017 The Rust Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A distribution uniformly sampling numbers within a given range.
//!
//! [`Uniform`] is the standard distribution to sample uniformly from a range;
//! e.g. `Uniform::new_inclusive(1, 6)` can sample integers from 1 to 6, like a
//! standard die. [`Rng::gen_range`] supports any type supported by
//! [`Uniform`].
//!
//! This distribution is provided with support for several primitive types
//! (all integer and floating-point types) as well as `std::time::Duration`,
//! and supports extension to user-defined types via a type-specific *back-end*
//! implementation.
//!
//! The types [`UniformInt`], [`UniformFloat`] and [`UniformDuration`] are the
//! back-ends supporting sampling from primitive integer and floating-point
//! ranges as well as from `std::time::Duration`; these types do not normally
//! need to be used directly (unless implementing a derived back-end).
//!
//! # Example usage
//!
//! ```
//! use rand::{Rng, thread_rng};
//! use rand::distributions::Uniform;
//!
//! let mut rng = thread_rng();
//! let side = Uniform::new(-10.0, 10.0);
//!
//! // sample between 1 and 10 points
//! for _ in 0..rng.gen_range(1, 11) {
//!     // sample a point from the square with sides -10 - 10 in two dimensions
//!     let (x, y) = (rng.sample(side), rng.sample(side));
//!     println!("Point: {}, {}", x, y);
//! }
//! ```
//!
//! # Extending `Uniform` to support a custom type
//!
//! To extend [`Uniform`] to support your own types, write a back-end which
//! implements the [`UniformSampler`] trait, then implement the [`SampleUniform`]
//! helper trait to "register" your back-end. See the `MyF32` example below.
//!
//! At a minimum, the back-end needs to store any parameters needed for sampling
//! (e.g. the target range) and implement `new`, `new_inclusive` and `sample`.
//! Those methods should include an assert to check the range is valid (i.e.
//! `low < high`). The example below merely wraps another back-end.
//!
//! The `new`, `new_inclusive` and `sample_single` functions use arguments of
//! type SampleBorrow<X> in order to support passing in values by reference or
//! by value. In the implementation of these functions, you can choose to
//! simply use the reference returned by [`SampleBorrow::borrow`], or you can choose
//! to copy or clone the value, whatever is appropriate for your type.
//!
//! ```
//! use rand::prelude::*;
//! use rand::distributions::uniform::{Uniform, SampleUniform,
//!         UniformSampler, UniformFloat, SampleBorrow};
//!
//! struct MyF32(f32);
//!
//! #[derive(Clone, Copy, Debug)]
//! struct UniformMyF32 {
//!     inner: UniformFloat<f32>,
//! }
//!
//! impl UniformSampler for UniformMyF32 {
//!     type X = MyF32;
//!     fn new<B1, B2>(low: B1, high: B2) -> Self
//!         where B1: SampleBorrow<Self::X> + Sized,
//!               B2: SampleBorrow<Self::X> + Sized
//!     {
//!         UniformMyF32 {
//!             inner: UniformFloat::<f32>::new(low.borrow().0, high.borrow().0),
//!         }
//!     }
//!     fn new_inclusive<B1, B2>(low: B1, high: B2) -> Self
//!         where B1: SampleBorrow<Self::X> + Sized,
//!               B2: SampleBorrow<Self::X> + Sized
//!     {
//!         UniformSampler::new(low, high)
//!     }
//!     fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Self::X {
//!         MyF32(self.inner.sample(rng))
//!     }
//! }
//!
//! impl SampleUniform for MyF32 {
//!     type Sampler = UniformMyF32;
//! }
//!
//! let (low, high) = (MyF32(17.0f32), MyF32(22.0f32));
//! let uniform = Uniform::new(low, high);
//! let x = uniform.sample(&mut thread_rng());
//! ```
//!
//! [`Uniform`]: struct.Uniform.html
//! [`Rng::gen_range`]: ../../trait.Rng.html#method.gen_range
//! [`SampleUniform`]: trait.SampleUniform.html
//! [`UniformSampler`]: trait.UniformSampler.html
//! [`UniformInt`]: struct.UniformInt.html
//! [`UniformFloat`]: struct.UniformFloat.html
//! [`UniformDuration`]: struct.UniformDuration.html
//! [`Borrow::borrow`]: trait.SampleBorrow.html

#[cfg(feature = "std")]
use std::time::Duration;

use Rng;
use distributions::Distribution;
use distributions::float::IntoFloat;
use distributions::utils::{WideningMultiply, FloatSIMDUtils, FloatAsSIMD, BoolAsSIMD};

#[cfg(not(feature = "std"))]
#[allow(unused_imports)] // rustc doesn't detect that this is actually used
use distributions::utils::Float;


#[cfg(feature="simd_support")]
use packed_simd::*;

/// Sample values uniformly between two bounds.
///
/// [`Uniform::new`] and [`Uniform::new_inclusive`] construct a uniform
/// distribution sampling from the given range; these functions may do extra
/// work up front to make sampling of multiple values faster.
///
/// When sampling from a constant range, many calculations can happen at
/// compile-time and all methods should be fast; for floating-point ranges and
/// the full range of integer types this should have comparable performance to
/// the `Standard` distribution.
///
/// Steps are taken to avoid bias which might be present in naive
/// implementations; for example `rng.gen::<u8>() % 170` samples from the range
/// `[0, 169]` but is twice as likely to select numbers less than 85 than other
/// values. Further, the implementations here give more weight to the high-bits
/// generated by the RNG than the low bits, since with some RNGs the low-bits
/// are of lower quality than the high bits.
///
/// Implementations must sample in `[low, high)` range for
/// `Uniform::new(low, high)`, i.e., excluding `high`. In particular care must
/// be taken to ensure that rounding never results values `< low` or `>= high`.
///
/// # Example
///
/// ```
/// use rand::distributions::{Distribution, Uniform};
///
/// fn main() {
///     let between = Uniform::from(10..10000);
///     let mut rng = rand::thread_rng();
///     let mut sum = 0;
///     for _ in 0..1000 {
///         sum += between.sample(&mut rng);
///     }
///     println!("{}", sum);
/// }
/// ```
///
/// [`Uniform::new`]: struct.Uniform.html#method.new
/// [`Uniform::new_inclusive`]: struct.Uniform.html#method.new_inclusive
/// [`new`]: struct.Uniform.html#method.new
/// [`new_inclusive`]: struct.Uniform.html#method.new_inclusive
#[derive(Clone, Copy, Debug)]
pub struct Uniform<X: SampleUniform> {
    inner: X::Sampler,
}

impl<X: SampleUniform> Uniform<X> {
    /// Create a new `Uniform` instance which samples uniformly from the half
    /// open range `[low, high)` (excluding `high`). Panics if `low >= high`.
    pub fn new<B1, B2>(low: B1, high: B2) -> Uniform<X>
        where B1: SampleBorrow<X> + Sized,
              B2: SampleBorrow<X> + Sized
    {
        Uniform { inner: X::Sampler::new(low, high) }
    }

    /// Create a new `Uniform` instance which samples uniformly from the closed
    /// range `[low, high]` (inclusive). Panics if `low > high`.
    pub fn new_inclusive<B1, B2>(low: B1, high: B2) -> Uniform<X>
        where B1: SampleBorrow<X> + Sized,
              B2: SampleBorrow<X> + Sized
    {
        Uniform { inner: X::Sampler::new_inclusive(low, high) }
    }
}

impl<X: SampleUniform> Distribution<X> for Uniform<X> {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> X {
        self.inner.sample(rng)
    }
}

/// Helper trait for creating objects using the correct implementation of
/// [`UniformSampler`] for the sampling type.
///
/// See the [module documentation] on how to implement [`Uniform`] range
/// sampling for a custom type.
///
/// [`UniformSampler`]: trait.UniformSampler.html
/// [module documentation]: index.html
/// [`Uniform`]: struct.Uniform.html
pub trait SampleUniform: Sized {
    /// The `UniformSampler` implementation supporting type `X`.
    type Sampler: UniformSampler<X = Self>;
}

/// Helper trait handling actual uniform sampling.
///
/// See the [module documentation] on how to implement [`Uniform`] range
/// sampling for a custom type.
///
/// Implementation of [`sample_single`] is optional, and is only useful when
/// the implementation can be faster than `Self::new(low, high).sample(rng)`.
///
/// [module documentation]: index.html
/// [`Uniform`]: struct.Uniform.html
/// [`sample_single`]: trait.UniformSampler.html#method.sample_single
pub trait UniformSampler: Sized {
    /// The type sampled by this implementation.
    type X;

    /// Construct self, with inclusive lower bound and exclusive upper bound
    /// `[low, high)`.
    ///
    /// Usually users should not call this directly but instead use
    /// `Uniform::new`, which asserts that `low < high` before calling this.
    fn new<B1, B2>(low: B1, high: B2) -> Self
        where B1: SampleBorrow<Self::X> + Sized,
              B2: SampleBorrow<Self::X> + Sized;

    /// Construct self, with inclusive bounds `[low, high]`.
    ///
    /// Usually users should not call this directly but instead use
    /// `Uniform::new_inclusive`, which asserts that `low <= high` before
    /// calling this.
    fn new_inclusive<B1, B2>(low: B1, high: B2) -> Self
        where B1: SampleBorrow<Self::X> + Sized,
              B2: SampleBorrow<Self::X> + Sized;

    /// Sample a value.
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Self::X;

    /// Sample a single value uniformly from a range with inclusive lower bound
    /// and exclusive upper bound `[low, high)`.
    ///
    /// Usually users should not call this directly but instead use
    /// `Uniform::sample_single`, which asserts that `low < high` before calling
    /// this.
    ///
    /// Via this method, implementations can provide a method optimized for
    /// sampling only a single value from the specified range. The default
    /// implementation simply calls `UniformSampler::new` then `sample` on the
    /// result.
    fn sample_single<R: Rng + ?Sized, B1, B2>(low: B1, high: B2, rng: &mut R)
        -> Self::X
        where B1: SampleBorrow<Self::X> + Sized,
              B2: SampleBorrow<Self::X> + Sized
    {
        let uniform: Self = UniformSampler::new(low, high);
        uniform.sample(rng)
    }
}

impl<X: SampleUniform> From<::core::ops::Range<X>> for Uniform<X> {
    fn from(r: ::core::ops::Range<X>) -> Uniform<X> {
        Uniform::new(r.start, r.end)
    }
}

#[cfg(rust_1_27)]
impl<X: SampleUniform> From<::core::ops::RangeInclusive<X>> for Uniform<X> {
    fn from(r: ::core::ops::RangeInclusive<X>) -> Uniform<X> {
        Uniform::new_inclusive(r.start(), r.end())
    }
}

/// Helper trait similar to [`Borrow`] but implemented
/// only for SampleUniform and references to SampleUniform in
/// order to resolve ambiguity issues.
///
/// [`Borrow`]: https://doc.rust-lang.org/std/borrow/trait.Borrow.html
pub trait SampleBorrow<Borrowed> {
    /// Immutably borrows from an owned value. See [`Borrow::borrow`]
    ///
    /// [`Borrow::borrow`]: https://doc.rust-lang.org/std/borrow/trait.Borrow.html#tymethod.borrow
    fn borrow(&self) -> &Borrowed;
}
impl<Borrowed> SampleBorrow<Borrowed> for Borrowed where Borrowed: SampleUniform {
    #[inline(always)]
    fn borrow(&self) -> &Borrowed { self }
}
impl<'a, Borrowed> SampleBorrow<Borrowed> for &'a Borrowed where Borrowed: SampleUniform {
    #[inline(always)]
   fn borrow(&self) -> &Borrowed { *self }
}

////////////////////////////////////////////////////////////////////////////////

// What follows are all back-ends.


/// The back-end implementing [`UniformSampler`] for integer types.
///
/// Unless you are implementing [`UniformSampler`] for your own type, this type
/// should not be used directly, use [`Uniform`] instead.
///
/// # Implementation notes
///
/// For a closed range, the number of possible numbers we should generate is
/// `range = (high - low + 1)`. It is not possible to end up with a uniform
/// distribution if we map *all* the random integers that can be generated to
/// this range. We have to map integers from a `zone` that is a multiple of the
/// range. The rest of the integers, that cause a bias, are rejected.
///
/// The problem with `range` is that to cover the full range of the type, it has
/// to store `unsigned_max + 1`, which can't be represented. But if the range
/// covers the full range of the type, no modulus is needed. A range of size 0
/// can't exist, so we use that to represent this special case. Wrapping
/// arithmetic even makes representing `unsigned_max + 1` as 0 simple.
///
/// We don't calculate `zone` directly, but first calculate the number of
/// integers to reject. To handle `unsigned_max + 1` not fitting in the type,
/// we use:
/// `ints_to_reject = (unsigned_max + 1) % range;`
/// `ints_to_reject = (unsigned_max - range + 1) % range;`
///
/// The smallest integer PRNGs generate is `u32`. That is why for small integer
/// sizes (`i8`/`u8` and `i16`/`u16`) there is an optimization: don't pick the
/// largest zone that can fit in the small type, but pick the largest zone that
/// can fit in an `u32`. `ints_to_reject` is always less than half the size of
/// the small integer. This means the first bit of `zone` is always 1, and so
/// are all the other preceding bits of a larger integer. The easiest way to
/// grow the `zone` for the larger type is to simply sign extend it.
///
/// An alternative to using a modulus is widening multiply: After a widening
/// multiply by `range`, the result is in the high word. Then comparing the low
/// word against `zone` makes sure our distribution is uniform.
///
/// [`UniformSampler`]: trait.UniformSampler.html
/// [`Uniform`]: struct.Uniform.html
#[derive(Clone, Copy, Debug)]
pub struct UniformInt<X> {
    low: X,
    range: X,
    zone: X,
}

macro_rules! uniform_int_impl {
    ($ty:ty, $signed:ty, $unsigned:ident,
     $i_large:ident, $u_large:ident) => {
        impl SampleUniform for $ty {
            type Sampler = UniformInt<$ty>;
        }

        impl UniformSampler for UniformInt<$ty> {
            // We play free and fast with unsigned vs signed here
            // (when $ty is signed), but that's fine, since the
            // contract of this macro is for $ty and $unsigned to be
            // "bit-equal", so casting between them is a no-op.

            type X = $ty;

            #[inline] // if the range is constant, this helps LLVM to do the
                      // calculations at compile-time.
            fn new<B1, B2>(low_b: B1, high_b: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low < high, "Uniform::new called with `low >= high`");
                UniformSampler::new_inclusive(low, high - 1)
            }

            #[inline] // if the range is constant, this helps LLVM to do the
                      // calculations at compile-time.
            fn new_inclusive<B1, B2>(low_b: B1, high_b: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low <= high,
                        "Uniform::new_inclusive called with `low > high`");
                let unsigned_max = ::core::$unsigned::MAX;

                let range = high.wrapping_sub(low).wrapping_add(1) as $unsigned;
                let ints_to_reject =
                    if range > 0 {
                        (unsigned_max - range + 1) % range
                    } else {
                        0
                    };
                let zone = unsigned_max - ints_to_reject;

                UniformInt {
                    low: low,
                    // These are really $unsigned values, but store as $ty:
                    range: range as $ty,
                    zone: zone as $ty
                }
            }

            fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Self::X {
                let range = self.range as $unsigned as $u_large;
                if range > 0 {
                    // Grow `zone` to fit a type of at least 32 bits, by
                    // sign-extending it (the first bit is always 1, so are all
                    // the preceding bits of the larger type).
                    // For types that already have the right size, all the
                    // casting is a no-op.
                    let zone = self.zone as $signed as $i_large as $u_large;
                    loop {
                        let v: $u_large = rng.gen();
                        let (hi, lo) = v.wmul(range);
                        if lo <= zone {
                            return self.low.wrapping_add(hi as $ty);
                        }
                    }
                } else {
                    // Sample from the entire integer range.
                    rng.gen()
                }
            }

            fn sample_single<R: Rng + ?Sized, B1, B2>(low_b: B1, high_b: B2, rng: &mut R)
                -> Self::X
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low < high,
                        "Uniform::sample_single called with low >= high");
                let range = high.wrapping_sub(low) as $unsigned as $u_large;
                let zone =
                    if ::core::$unsigned::MAX <= ::core::u16::MAX as $unsigned {
                        // Using a modulus is faster than the approximation for
                        // i8 and i16. I suppose we trade the cost of one
                        // modulus for near-perfect branch prediction.
                        let unsigned_max: $u_large = ::core::$u_large::MAX;
                        let ints_to_reject = (unsigned_max - range + 1) % range;
                        unsigned_max - ints_to_reject
                    } else {
                        // conservative but fast approximation
                       range << range.leading_zeros()
                    };

                loop {
                    let v: $u_large = rng.gen();
                    let (hi, lo) = v.wmul(range);
                    if lo <= zone {
                        return low.wrapping_add(hi as $ty);
                    }
                }
            }
        }
    }
}

uniform_int_impl! { i8, i8, u8, i32, u32 }
uniform_int_impl! { i16, i16, u16, i32, u32 }
uniform_int_impl! { i32, i32, u32, i32, u32 }
uniform_int_impl! { i64, i64, u64, i64, u64 }
#[cfg(rust_1_26)]
uniform_int_impl! { i128, i128, u128, u128, u128 }
uniform_int_impl! { isize, isize, usize, isize, usize }
uniform_int_impl! { u8, i8, u8, i32, u32 }
uniform_int_impl! { u16, i16, u16, i32, u32 }
uniform_int_impl! { u32, i32, u32, i32, u32 }
uniform_int_impl! { u64, i64, u64, i64, u64 }
uniform_int_impl! { usize, isize, usize, isize, usize }
#[cfg(rust_1_26)]
uniform_int_impl! { u128, u128, u128, i128, u128 }

#[cfg(feature = "simd_support")]
macro_rules! uniform_simd_int_impl {
    ($ty:ident, $unsigned:ident, $u_scalar:ident) => {
        // The "pick the largest zone that can fit in an `u32`" optimization
        // is less useful here. Multiple lanes complicate things, we don't
        // know the PRNG's minimal output size, and casting to a larger vector
        // is generally a bad idea for SIMD performance. The user can still
        // implement it manually.

        // TODO: look into `Uniform::<u32x4>::new(0u32, 100)` functionality
        //       perhaps `impl SampleUniform for $u_scalar`?
        impl SampleUniform for $ty {
            type Sampler = UniformInt<$ty>;
        }

        impl UniformSampler for UniformInt<$ty> {
            type X = $ty;

            #[inline] // if the range is constant, this helps LLVM to do the
                      // calculations at compile-time.
            fn new<B1, B2>(low_b: B1, high_b: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low.lt(high).all(), "Uniform::new called with `low >= high`");
                UniformSampler::new_inclusive(low, high - 1)
            }

            #[inline] // if the range is constant, this helps LLVM to do the
                      // calculations at compile-time.
            fn new_inclusive<B1, B2>(low_b: B1, high_b: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low.le(high).all(),
                        "Uniform::new_inclusive called with `low > high`");
                let unsigned_max = ::core::$u_scalar::MAX;

                // NOTE: these may need to be replaced with explicitly
                // wrapping operations if `packed_simd` changes
                let range: $unsigned = ((high - low) + 1).cast();
                // `% 0` will panic at runtime.
                let not_full_range = range.gt($unsigned::splat(0));
                // replacing 0 with `unsigned_max` allows a faster `select`
                // with bitwise OR
                let modulo = not_full_range.select(range, $unsigned::splat(unsigned_max));
                // wrapping addition
                let ints_to_reject = (unsigned_max - range + 1) % modulo;
                // When `range` is 0, `lo` of `v.wmul(range)` will always be
                // zero which means only one sample is needed.
                let zone = unsigned_max - ints_to_reject;

                UniformInt {
                    low: low,
                    // These are really $unsigned values, but store as $ty:
                    range: range.cast(),
                    zone: zone.cast(),
                }
            }

            fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Self::X {
                let range: $unsigned = self.range.cast();
                let zone: $unsigned = self.zone.cast();

                // This might seem very slow, generating a whole new
                // SIMD vector for every sample rejection. For most uses
                // though, the chance of rejection is small and provides good
                // general performance. With multiple lanes, that chance is
                // multiplied. To mitigate this, we replace only the lanes of
                // the vector which fail, iteratively reducing the chance of
                // rejection. The replacement method does however add a little
                // overhead. Benchmarking or calculating probabilities might
                // reveal contexts where this replacement method is slower.
                let mut v: $unsigned = rng.gen();
                loop {
                    let (hi, lo) = v.wmul(range);
                    let mask = lo.le(zone);
                    if mask.all() {
                        let hi: $ty = hi.cast();
                        // wrapping addition
                        let result = self.low + hi;
                        // `select` here compiles to a blend operation
                        // When `range.eq(0).none()` the compare and blend
                        // operations are avoided.
                        let v: $ty = v.cast();
                        return range.gt($unsigned::splat(0)).select(result, v);
                    }
                    // Replace only the failing lanes
                    v = mask.select(v, rng.gen());
                }
            }
        }
    };

    // bulk implementation
    ($(($unsigned:ident, $signed:ident),)+ $u_scalar:ident) => {
        $(
            uniform_simd_int_impl!($unsigned, $unsigned, $u_scalar);
            uniform_simd_int_impl!($signed, $unsigned, $u_scalar);
        )+
    };
}

#[cfg(feature = "simd_support")]
uniform_simd_int_impl! {
    (u64x2, i64x2),
    (u64x4, i64x4),
    (u64x8, i64x8),
    u64
}

#[cfg(feature = "simd_support")]
uniform_simd_int_impl! {
    (u32x2, i32x2),
    (u32x4, i32x4),
    (u32x8, i32x8),
    (u32x16, i32x16),
    u32
}

#[cfg(feature = "simd_support")]
uniform_simd_int_impl! {
    (u16x2, i16x2),
    (u16x4, i16x4),
    (u16x8, i16x8),
    (u16x16, i16x16),
    (u16x32, i16x32),
    u16
}

#[cfg(feature = "simd_support")]
uniform_simd_int_impl! {
    (u8x2, i8x2),
    (u8x4, i8x4),
    (u8x8, i8x8),
    (u8x16, i8x16),
    (u8x32, i8x32),
    (u8x64, i8x64),
    u8
}


/// The back-end implementing [`UniformSampler`] for floating-point types.
///
/// Unless you are implementing [`UniformSampler`] for your own type, this type
/// should not be used directly, use [`Uniform`] instead.
///
/// # Implementation notes
///
/// Instead of generating a float in the `[0, 1)` range using [`Standard`], the
/// `UniformFloat` implementation converts the output of an PRNG itself. This
/// way one or two steps can be optimized out.
///
/// The floats are first converted to a value in the `[1, 2)` interval using a
/// transmute-based method, and then mapped to the expected range with a
/// multiply and addition. Values produced this way have what equals 22 bits of
/// random digits for an `f32`, and 52 for an `f64`.
///
/// [`UniformSampler`]: trait.UniformSampler.html
/// [`new`]: trait.UniformSampler.html#tymethod.new
/// [`new_inclusive`]: trait.UniformSampler.html#tymethod.new_inclusive
/// [`Uniform`]: struct.Uniform.html
/// [`Standard`]: ../struct.Standard.html
#[derive(Clone, Copy, Debug)]
pub struct UniformFloat<X> {
    low: X,
    scale: X,
}

macro_rules! uniform_float_impl {
    ($ty:ty, $uty:ident, $f_scalar:ident, $u_scalar:ident, $bits_to_discard:expr) => {
        impl SampleUniform for $ty {
            type Sampler = UniformFloat<$ty>;
        }

        impl UniformSampler for UniformFloat<$ty> {
            type X = $ty;

            fn new<B1, B2>(low_b: B1, high_b: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low.all_lt(high),
                        "Uniform::new called with `low >= high`");
                assert!(low.all_finite() && high.all_finite(),
                        "Uniform::new called with non-finite boundaries");
                let max_rand = <$ty>::splat((::core::$u_scalar::MAX >> $bits_to_discard)
                                            .into_float_with_exponent(0) - 1.0);

                let mut scale = high - low;

                loop {
                    let mask = (scale * max_rand + low).ge_mask(high);
                    if mask.none() {
                        break;
                    }
                    scale = scale.decrease_masked(mask);
                }

                debug_assert!(<$ty>::splat(0.0).all_le(scale));

                UniformFloat { low, scale }
            }

            fn new_inclusive<B1, B2>(low_b: B1, high_b: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low.all_le(high),
                        "Uniform::new_inclusive called with `low > high`");
                assert!(low.all_finite() && high.all_finite(),
                        "Uniform::new_inclusive called with non-finite boundaries");
                let max_rand = <$ty>::splat((::core::$u_scalar::MAX >> $bits_to_discard)
                                            .into_float_with_exponent(0) - 1.0);

                let mut scale = (high - low) / max_rand;

                loop {
                    let mask = (scale * max_rand + low).gt_mask(high);
                    if mask.none() {
                        break;
                    }
                    scale = scale.decrease_masked(mask);
                }

                debug_assert!(<$ty>::splat(0.0).all_le(scale));

                UniformFloat { low, scale }
            }

            fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Self::X {
                // Generate a value in the range [1, 2)
                let value1_2 = (rng.gen::<$uty>() >> $bits_to_discard)
                               .into_float_with_exponent(0);

                // Get a value in the range [0, 1) in order to avoid
                // overflowing into infinity when multiplying with scale
                let value0_1 = value1_2 - 1.0;

                // We don't use `f64::mul_add`, because it is not available with
                // `no_std`. Furthermore, it is slower for some targets (but
                // faster for others). However, the order of multiplication and
                // addition is important, because on some platforms (e.g. ARM)
                // it will be optimized to a single (non-FMA) instruction.
                value0_1 * self.scale + self.low
            }

            #[inline]
            fn sample_single<R: Rng + ?Sized, B1, B2>(low_b: B1, high_b: B2, rng: &mut R)
                -> Self::X
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                let low = *low_b.borrow();
                let high = *high_b.borrow();
                assert!(low.all_lt(high),
                        "Uniform::sample_single called with low >= high");
                let mut scale = high - low;

                loop {
                    // Generate a value in the range [1, 2)
                    let value1_2 = (rng.gen::<$uty>() >> $bits_to_discard)
                                   .into_float_with_exponent(0);

                    // Get a value in the range [0, 1) in order to avoid
                    // overflowing into infinity when multiplying with scale
                    let value0_1 = value1_2 - 1.0;

                    // Doing multiply before addition allows some architectures
                    // to use a single instruction.
                    let res = value0_1 * scale + low;

                    debug_assert!(low.all_le(res) || !scale.all_finite());
                    if res.all_lt(high) {
                        return res;
                    }

                    // This handles a number of edge cases.
                    // * `low` or `high` is NaN. In this case `scale` and
                    //   `res` are going to end up as NaN.
                    // * `low` is negative infinity and `high` is finite.
                    //   `scale` is going to be infinite and `res` will be
                    //   NaN.
                    // * `high` is positive infinity and `low` is finite.
                    //   `scale` is going to be infinite and `res` will
                    //   be infinite or NaN (if value0_1 is 0).
                    // * `low` is negative infinity and `high` is positive
                    //   infinity. `scale` will be infinite and `res` will
                    //   be NaN.
                    // * `low` and `high` are finite, but `high - low`
                    //   overflows to infinite. `scale` will be infinite
                    //   and `res` will be infinite or NaN (if value0_1 is 0).
                    // So if `high` or `low` are non-finite, we are guaranteed
                    // to fail the `res < high` check above and end up here.
                    //
                    // While we technically should check for non-finite `low`
                    // and `high` before entering the loop, by doing the checks
                    // here instead, we allow the common case to avoid these
                    // checks. But we are still guaranteed that if `low` or
                    // `high` are non-finite we'll end up here and can do the
                    // appropriate checks.
                    //
                    // Likewise `high - low` overflowing to infinity is also
                    // rare, so handle it here after the common case.
                    let mask = !scale.finite_mask();
                    if mask.any() {
                        assert!(low.all_finite() && high.all_finite(),
                                "Uniform::sample_single called with non-finite boundaries");
                        scale = scale.decrease_masked(mask);
                    }
                }
            }
        }
    }
}

uniform_float_impl! { f32, u32, f32, u32, 32 - 23 }
uniform_float_impl! { f64, u64, f64, u64, 64 - 52 }

#[cfg(feature="simd_support")]
uniform_float_impl! { f32x2, u32x2, f32, u32, 32 - 23 }
#[cfg(feature="simd_support")]
uniform_float_impl! { f32x4, u32x4, f32, u32, 32 - 23 }
#[cfg(feature="simd_support")]
uniform_float_impl! { f32x8, u32x8, f32, u32, 32 - 23 }
#[cfg(feature="simd_support")]
uniform_float_impl! { f32x16, u32x16, f32, u32, 32 - 23 }

#[cfg(feature="simd_support")]
uniform_float_impl! { f64x2, u64x2, f64, u64, 64 - 52 }
#[cfg(feature="simd_support")]
uniform_float_impl! { f64x4, u64x4, f64, u64, 64 - 52 }
#[cfg(feature="simd_support")]
uniform_float_impl! { f64x8, u64x8, f64, u64, 64 - 52 }



/// The back-end implementing [`UniformSampler`] for `Duration`.
///
/// Unless you are implementing [`UniformSampler`] for your own types, this type
/// should not be used directly, use [`Uniform`] instead.
///
/// [`UniformSampler`]: trait.UniformSampler.html
/// [`Uniform`]: struct.Uniform.html
#[cfg(feature = "std")]
#[derive(Clone, Copy, Debug)]
pub struct UniformDuration {
    offset: Duration,
    mode: UniformDurationMode,
}

#[cfg(feature = "std")]
#[derive(Debug, Copy, Clone)]
enum UniformDurationMode {
    Small {
        nanos: Uniform<u64>,
    },
    Large {
        size: Duration,
        secs: Uniform<u64>,
    }
}

#[cfg(feature = "std")]
impl SampleUniform for Duration {
    type Sampler = UniformDuration;
}

#[cfg(feature = "std")]
impl UniformSampler for UniformDuration {
    type X = Duration;

    #[inline]
    fn new<B1, B2>(low_b: B1, high_b: B2) -> Self
        where B1: SampleBorrow<Self::X> + Sized,
              B2: SampleBorrow<Self::X> + Sized
    {
        let low = *low_b.borrow();
        let high = *high_b.borrow();
        assert!(low < high, "Uniform::new called with `low >= high`");
        UniformDuration::new_inclusive(low, high - Duration::new(0, 1))
    }

    #[inline]
    fn new_inclusive<B1, B2>(low_b: B1, high_b: B2) -> Self
        where B1: SampleBorrow<Self::X> + Sized,
              B2: SampleBorrow<Self::X> + Sized
    {
        let low = *low_b.borrow();
        let high = *high_b.borrow();
        assert!(low <= high, "Uniform::new_inclusive called with `low > high`");
        let size = high - low;
        let nanos = size
            .as_secs()
            .checked_mul(1_000_000_000)
            .and_then(|n| n.checked_add(size.subsec_nanos() as u64));

        let mode = match nanos {
            Some(nanos) => {
                UniformDurationMode::Small {
                    nanos: Uniform::new_inclusive(0, nanos),
                }
            }
            None => {
                UniformDurationMode::Large {
                    size: size,
                    secs: Uniform::new_inclusive(0, size.as_secs()),
                }
            }
        };

        UniformDuration {
            mode,
            offset: low,
        }
    }

    #[inline]
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Duration {
        let d = match self.mode {
            UniformDurationMode::Small { nanos } => {
                let nanos = nanos.sample(rng);
                Duration::new(nanos / 1_000_000_000, (nanos % 1_000_000_000) as u32)
            }
            UniformDurationMode::Large { size, secs } => {
                // constant folding means this is at least as fast as `gen_range`
                let nano_range = Uniform::new(0, 1_000_000_000);
                loop {
                    let d = Duration::new(secs.sample(rng), nano_range.sample(rng));
                    if d <= size {
                        break d;
                    }
                }
            }
        };

        self.offset + d
    }
}

#[cfg(test)]
mod tests {
    use Rng;
    use rngs::mock::StepRng;
    use distributions::uniform::Uniform;
    use distributions::utils::FloatAsSIMD;
    #[cfg(feature="simd_support")] use packed_simd::*;

    #[should_panic]
    #[test]
    fn test_uniform_bad_limits_equal_int() {
        Uniform::new(10, 10);
    }

    #[test]
    fn test_uniform_good_limits_equal_int() {
        let mut rng = ::test::rng(804);
        let dist = Uniform::new_inclusive(10, 10);
        for _ in 0..20 {
            assert_eq!(rng.sample(dist), 10);
        }
    }

    #[should_panic]
    #[test]
    fn test_uniform_bad_limits_flipped_int() {
        Uniform::new(10, 5);
    }

    #[test]
    fn test_integers() {
        use core::{i8, i16, i32, i64, isize};
        use core::{u8, u16, u32, u64, usize};
        #[cfg(rust_1_26)]
        use core::{i128, u128};

        let mut rng = ::test::rng(251);
        macro_rules! t {
            ($ty:ident, $v:expr, $le:expr, $lt:expr) => {{
                for &(low, high) in $v.iter() {
                    let my_uniform = Uniform::new(low, high);
                    for _ in 0..1000 {
                        let v: $ty = rng.sample(my_uniform);
                        assert!($le(low, v) && $lt(v, high));
                    }

                    let my_uniform = Uniform::new_inclusive(low, high);
                    for _ in 0..1000 {
                        let v: $ty = rng.sample(my_uniform);
                        assert!($le(low, v) && $le(v, high));
                    }

                    let my_uniform = Uniform::new(&low, high);
                    for _ in 0..1000 {
                        let v: $ty = rng.sample(my_uniform);
                        assert!($le(low, v) && $lt(v, high));
                    }

                    let my_uniform = Uniform::new_inclusive(&low, &high);
                    for _ in 0..1000 {
                        let v: $ty = rng.sample(my_uniform);
                        assert!($le(low, v) && $le(v, high));
                    }

                    for _ in 0..1000 {
                        let v: $ty = rng.gen_range(low, high);
                        assert!($le(low, v) && $lt(v, high));
                    }
                }
            }};

            // scalar bulk
            ($($ty:ident),*) => {{
                $(t!(
                    $ty,
                    [(0, 10), (10, 127), ($ty::MIN, $ty::MAX)],
                    |x, y| x <= y,
                    |x, y| x < y
                );)*
            }};

            // simd bulk
            ($($ty:ident),* => $scalar:ident) => {{
                $(t!(
                    $ty,
                    [
                        ($ty::splat(0), $ty::splat(10)),
                        ($ty::splat(10), $ty::splat(127)),
                        ($ty::splat($scalar::MIN), $ty::splat($scalar::MAX)),
                    ],
                    |x: $ty, y| x.le(y).all(),
                    |x: $ty, y| x.lt(y).all()
                );)*
            }};
        }
        t!(i8, i16, i32, i64, isize,
           u8, u16, u32, u64, usize);
        #[cfg(rust_1_26)]
        t!(i128, u128);

        #[cfg(feature = "simd_support")]
        {
            t!(u8x2, u8x4, u8x8, u8x16, u8x32, u8x64 => u8);
            t!(i8x2, i8x4, i8x8, i8x16, i8x32, i8x64 => i8);
            t!(u16x2, u16x4, u16x8, u16x16, u16x32 => u16);
            t!(i16x2, i16x4, i16x8, i16x16, i16x32 => i16);
            t!(u32x2, u32x4, u32x8, u32x16 => u32);
            t!(i32x2, i32x4, i32x8, i32x16 => i32);
            t!(u64x2, u64x4, u64x8 => u64);
            t!(i64x2, i64x4, i64x8 => i64);
        }
    }

    #[test]
    fn test_floats() {
        let mut rng = ::test::rng(252);
        let mut zero_rng = StepRng::new(0, 0);
        let mut max_rng = StepRng::new(0xffff_ffff_ffff_ffff, 0);
        macro_rules! t {
            ($ty:ty, $f_scalar:ident, $bits_shifted:expr) => {{
                let v: &[($f_scalar, $f_scalar)]=
                    &[(0.0, 100.0),
                      (-1e35, -1e25),
                      (1e-35, 1e-25),
                      (-1e35, 1e35),
                      (<$f_scalar>::from_bits(0), <$f_scalar>::from_bits(3)),
                      (-<$f_scalar>::from_bits(10), -<$f_scalar>::from_bits(1)),
                      (-<$f_scalar>::from_bits(5), 0.0),
                      (-<$f_scalar>::from_bits(7), -0.0),
                      (10.0, ::core::$f_scalar::MAX),
                      (-100.0, ::core::$f_scalar::MAX),
                      (-::core::$f_scalar::MAX / 5.0, ::core::$f_scalar::MAX),
                      (-::core::$f_scalar::MAX, ::core::$f_scalar::MAX / 5.0),
                      (-::core::$f_scalar::MAX * 0.8, ::core::$f_scalar::MAX * 0.7),
                      (-::core::$f_scalar::MAX, ::core::$f_scalar::MAX),
                     ];
                for &(low_scalar, high_scalar) in v.iter() {
                    for lane in 0..<$ty>::lanes() {
                        let low = <$ty>::splat(0.0 as $f_scalar).replace(lane, low_scalar);
                        let high = <$ty>::splat(1.0 as $f_scalar).replace(lane, high_scalar);
                        let my_uniform = Uniform::new(low, high);
                        let my_incl_uniform = Uniform::new_inclusive(low, high);
                        for _ in 0..100 {
                            let v = rng.sample(my_uniform).extract(lane);
                            assert!(low_scalar <= v && v < high_scalar);
                            let v = rng.sample(my_incl_uniform).extract(lane);
                            assert!(low_scalar <= v && v <= high_scalar);
                            let v = rng.gen_range(low, high).extract(lane);
                            assert!(low_scalar <= v && v < high_scalar);
                        }

                        assert_eq!(rng.sample(Uniform::new_inclusive(low, low)).extract(lane), low_scalar);

                        assert_eq!(zero_rng.sample(my_uniform).extract(lane), low_scalar);
                        assert_eq!(zero_rng.sample(my_incl_uniform).extract(lane), low_scalar);
                        assert_eq!(zero_rng.gen_range(low, high).extract(lane), low_scalar);
                        assert!(max_rng.sample(my_uniform).extract(lane) < high_scalar);
                        assert!(max_rng.sample(my_incl_uniform).extract(lane) <= high_scalar);

                        // Don't run this test for really tiny differences between high and low
                        // since for those rounding might result in selecting high for a very
                        // long time.
                        if (high_scalar - low_scalar) > 0.0001 {
                            let mut lowering_max_rng =
                                StepRng::new(0xffff_ffff_ffff_ffff,
                                             (-1i64 << $bits_shifted) as u64);
                            assert!(lowering_max_rng.gen_range(low, high).extract(lane) < high_scalar);
                        }
                    }
                }

                assert_eq!(rng.sample(Uniform::new_inclusive(::core::$f_scalar::MAX,
                                                             ::core::$f_scalar::MAX)),
                           ::core::$f_scalar::MAX);
                assert_eq!(rng.sample(Uniform::new_inclusive(-::core::$f_scalar::MAX,
                                                             -::core::$f_scalar::MAX)),
                           -::core::$f_scalar::MAX);
            }}
        }

        t!(f32, f32, 32 - 23);
        t!(f64, f64, 64 - 52);
        #[cfg(feature="simd_support")]
        {
            t!(f32x2, f32, 32 - 23);
            t!(f32x4, f32, 32 - 23);
            t!(f32x8, f32, 32 - 23);
            t!(f32x16, f32, 32 - 23);
            t!(f64x2, f64, 64 - 52);
            t!(f64x4, f64, 64 - 52);
            t!(f64x8, f64, 64 - 52);
        }
    }

    #[test]
    #[cfg(all(feature="std",
              not(target_arch = "wasm32"),
              not(target_arch = "asmjs")))]
    fn test_float_assertions() {
        use std::panic::catch_unwind;
        use super::SampleUniform;
        fn range<T: SampleUniform>(low: T, high: T) {
            let mut rng = ::test::rng(253);
            rng.gen_range(low, high);
        }

        macro_rules! t {
            ($ty:ident, $f_scalar:ident) => {{
                let v: &[($f_scalar, $f_scalar)] =
                    &[(::std::$f_scalar::NAN, 0.0),
                      (1.0, ::std::$f_scalar::NAN),
                      (::std::$f_scalar::NAN, ::std::$f_scalar::NAN),
                      (1.0, 0.5),
                      (::std::$f_scalar::MAX, -::std::$f_scalar::MAX),
                      (::std::$f_scalar::INFINITY, ::std::$f_scalar::INFINITY),
                      (::std::$f_scalar::NEG_INFINITY, ::std::$f_scalar::NEG_INFINITY),
                      (::std::$f_scalar::NEG_INFINITY, 5.0),
                      (5.0, ::std::$f_scalar::INFINITY),
                      (::std::$f_scalar::NAN, ::std::$f_scalar::INFINITY),
                      (::std::$f_scalar::NEG_INFINITY, ::std::$f_scalar::NAN),
                      (::std::$f_scalar::NEG_INFINITY, ::std::$f_scalar::INFINITY),
                     ];
                for &(low_scalar, high_scalar) in v.iter() {
                    for lane in 0..<$ty>::lanes() {
                        let low = <$ty>::splat(0.0 as $f_scalar).replace(lane, low_scalar);
                        let high = <$ty>::splat(1.0 as $f_scalar).replace(lane, high_scalar);
                        assert!(catch_unwind(|| range(low, high)).is_err());
                        assert!(catch_unwind(|| Uniform::new(low, high)).is_err());
                        assert!(catch_unwind(|| Uniform::new_inclusive(low, high)).is_err());
                        assert!(catch_unwind(|| range(low, low)).is_err());
                        assert!(catch_unwind(|| Uniform::new(low, low)).is_err());
                    }
                }
            }}
        }

        t!(f32, f32);
        t!(f64, f64);
        #[cfg(feature="simd_support")]
        {
            t!(f32x2, f32);
            t!(f32x4, f32);
            t!(f32x8, f32);
            t!(f32x16, f32);
            t!(f64x2, f64);
            t!(f64x4, f64);
            t!(f64x8, f64);
        }
    }


    #[test]
    #[cfg(feature = "std")]
    fn test_durations() {
        use std::time::Duration;

        let mut rng = ::test::rng(253);

        let v = &[(Duration::new(10, 50000), Duration::new(100, 1234)),
                  (Duration::new(0, 100), Duration::new(1, 50)),
                  (Duration::new(0, 0), Duration::new(u64::max_value(), 999_999_999))];
        for &(low, high) in v.iter() {
            let my_uniform = Uniform::new(low, high);
            for _ in 0..1000 {
                let v = rng.sample(my_uniform);
                assert!(low <= v && v < high);
            }
        }
    }

    #[test]
    fn test_custom_uniform() {
        use distributions::uniform::{UniformSampler, UniformFloat, SampleUniform, SampleBorrow};
        #[derive(Clone, Copy, PartialEq, PartialOrd)]
        struct MyF32 {
            x: f32,
        }
        #[derive(Clone, Copy, Debug)]
        struct UniformMyF32 {
            inner: UniformFloat<f32>,
        }
        impl UniformSampler for UniformMyF32 {
            type X = MyF32;
            fn new<B1, B2>(low: B1, high: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                UniformMyF32 {
                    inner: UniformFloat::<f32>::new(low.borrow().x, high.borrow().x),
                }
            }
            fn new_inclusive<B1, B2>(low: B1, high: B2) -> Self
                where B1: SampleBorrow<Self::X> + Sized,
                      B2: SampleBorrow<Self::X> + Sized
            {
                UniformSampler::new(low, high)
            }
            fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Self::X {
                MyF32 { x: self.inner.sample(rng) }
            }
        }
        impl SampleUniform for MyF32 {
            type Sampler = UniformMyF32;
        }

        let (low, high) = (MyF32{ x: 17.0f32 }, MyF32{ x: 22.0f32 });
        let uniform = Uniform::new(low, high);
        let mut rng = ::test::rng(804);
        for _ in 0..100 {
            let x: MyF32 = rng.sample(uniform);
            assert!(low <= x && x < high);
        }
    }

    #[test]
    fn test_uniform_from_std_range() {
        let r = Uniform::from(2u32..7);
        assert_eq!(r.inner.low, 2);
        assert_eq!(r.inner.range, 5);
        let r = Uniform::from(2.0f64..7.0);
        assert_eq!(r.inner.low, 2.0);
        assert_eq!(r.inner.scale, 5.0);
    }

    #[cfg(rust_1_27)]
    #[test]
    fn test_uniform_from_std_range_inclusive() {
        let r = Uniform::from(2u32..=6);
        assert_eq!(r.inner.low, 2);
        assert_eq!(r.inner.range, 5);
        let r = Uniform::from(2.0f64..=7.0);
        assert_eq!(r.inner.low, 2.0);
        assert!(r.inner.scale > 5.0);
        assert!(r.inner.scale < 5.0 + 1e-14);
    }
}
