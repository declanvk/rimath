use crate::{error::Error, integer::Integer};
use core::{convert::TryFrom, fmt, mem, mem::MaybeUninit, ptr, ptr::NonNull, str::FromStr};
use std::{alloc, ffi::CString, os::raw::c_long};

pub(crate) mod ops;

/// Multiple precision rational value. Always heap allocated, not safe for
/// sharing across threads.
#[repr(transparent)]
pub struct Rational {
    // This value must be constructed from a Box and then when Drop, must be reconstructed so that
    // the Box Drop can free the memory used.
    raw: NonNull<imath_sys::mpq_t>,
}

impl Rational {
    pub(crate) fn uninit() -> Box<MaybeUninit<imath_sys::mpq_t>> {
        // Replace with Box::new_uninit when it is stable (1.41 maybe?).
        let layout = alloc::Layout::new::<MaybeUninit<imath_sys::mpq_t>>();
        let ptr = unsafe { alloc::alloc(layout) };
        // This cast is safe bc the layout was specified for
        // MaybeUninit<imath_sys::mpq_t>
        unsafe { Box::from_raw(ptr.cast()) }
    }

    /// Create a new rational with a default value of zero (0/1).
    pub fn new() -> Self {
        let mut init = Rational::uninit();

        {
            let raw_mpq = init.as_mut_ptr();

            // This function call is safe because the mp_rat_init function only requires
            // that the mpq_t struct has been correctly allocated.
            let res = unsafe { imath_sys::mp_rat_init(raw_mpq) };

            imath_check_panic!(res, "Rational init failed!");
        }

        // This cast is safe (from MaybeUninit<imath_sys::mpq_t> to imath_sys::mpq_t)
        // because the value is now initialized.
        unsafe { Rational::from_raw(Box::into_raw(init).cast()) }
    }

    pub(crate) fn copy_init(other: &Self) -> Self {
        let mut init = Rational::uninit();
        let other_raw = other.as_raw();

        {
            let raw_mpq = init.as_mut_ptr();

            // This function call is safe because the mpq_t struct has been correctly
            // allocated and `other_raw` has been successfully initialized.
            let res = unsafe { imath_sys::mp_rat_init_copy(raw_mpq, other_raw) };

            imath_check_panic!(res, "Rational init failed!");
        }

        // This cast is safe (from MaybeUninit<imath_sys::mpq_t> to imath_sys::mpq_t)
        // because the value is now initialized.
        unsafe { Rational::from_raw(Box::into_raw(init).cast()) }
    }

    /// Construct a Rational from a raw non-null pointer to `imath_sys::mpq_t`.
    ///
    /// # Safety
    ///
    /// This function must only every be called once for a given pointer, and
    /// the pointer must point to an initialized `imath_sys::mpq_t` struct. The
    /// recommendation is to only use raw pointers from the `Rational::into_raw`
    /// function.
    ///
    /// In ths context, initialized means that the `imath_sys::mpq_t` has been
    /// the argument of a call to `imath_sys::mp_rat_init`.
    pub unsafe fn from_raw(raw: *mut imath_sys::mpq_t) -> Self {
        assert!(!raw.is_null());

        // This is safe bc the invariants of the function and because it was checked
        // that the pointer is not null.
        #[allow(unused_unsafe)]
        let raw = unsafe { NonNull::new_unchecked(raw) };

        Rational { raw }
    }

    /// Consumes the Rational, returning a wrapped raw pointer.
    pub fn into_raw(mut rational: Rational) -> *mut imath_sys::mpq_t {
        let raw = mem::replace(&mut rational.raw, NonNull::dangling());

        // The destructor does not need to run, as we are intentionally leaking the
        // resources here.
        mem::forget(rational);

        raw.as_ptr()
    }

    // Internal use only
    pub(crate) fn as_raw(&self) -> *mut imath_sys::mpq_t {
        self.raw.as_ptr()
    }

    /// Reduces `r` in-place to lowest terms and canonical form.
    ///
    /// Zero is represented as 0/1, one as 1/1, and signs are adjusted so that
    /// the sign of the value is carried by the numerator.
    pub fn reduce(&mut self) {
        let self_raw = self.as_raw();

        let res = unsafe { imath_sys::mp_rat_reduce(self_raw) };

        imath_check_panic!(res, "Reducing rational value failed!");
    }

    /// Return a copy of the numerator of the rational value
    pub fn numerator(&self) -> Integer {
        let mut numer = Integer::new();
        let self_raw = self.as_raw();

        {
            let raw_int = Integer::into_raw(numer);

            let res = unsafe { imath_sys::mp_rat_numer(self_raw, raw_int) };

            imath_check_panic!(res, "Value init failed!");

            // This is safe because the `raw_int` ptr is created directly from an
            // `Integer::into_raw` call.
            numer = unsafe { Integer::from_raw(raw_int) };
        }

        numer
    }

    /// Return a copy of the denominator of the rational value
    pub fn denominator(&self) -> Integer {
        let mut denom = Integer::new();
        let self_raw = self.as_raw();

        {
            let raw_int = Integer::into_raw(denom);

            // This function call is safe as self_raw and raw_int have been initialized.
            let res = unsafe { imath_sys::mp_rat_denom(self_raw, raw_int) };

            imath_check_panic!(res, "Value init failed!");

            // This is safe because the `raw_int` ptr is created directly from an
            // `Integer::into_raw` call.
            denom = unsafe { Integer::from_raw(raw_int) };
        }

        denom
    }

    /// Replaces the value of `other` with a copy of the value of `self`. No new
    /// memory is allocated unless `self` has more significant digits than
    /// `other` has allocated.
    pub fn copy_to(&self, other: &mut Self) {
        let self_raw = self.as_raw();
        let other_raw = other.as_raw();

        // This is safe bc self has been initialized with a value
        let res = unsafe { imath_sys::mp_rat_copy(other_raw, self_raw) };

        imath_check_panic!(res, "Copying the value failed!");
    }

    pub(crate) fn from_string_repr(src: &str) -> Result<Self, Error> {
        let string_repr =
            CString::new(src.to_string()).map_err(|_| Error::StringReprContainedNul)?;
        let char_ptr = string_repr.into_raw();

        let mut init = Rational::uninit();

        {
            let raw_mpq = init.as_mut_ptr();

            // This is safe bc a valid structure is provided to the unsafe methods. And the
            // src value is of the correct type?
            let res_init = unsafe { imath_sys::mp_rat_init(raw_mpq) };

            imath_check_panic!(res_init, "Init failed!");

            // This is safe bc all the data provided to the function is correctly setup
            // (rational was allocated/initialized, char_ptr is 0-terminated).
            let res_read =
                unsafe { imath_sys::mp_rat_read_ustring(raw_mpq, 10, char_ptr, ptr::null_mut()) };

            // Accessing this is safe bc the MP_OK value is only ever used as an error
            // condition.
            if res_read != unsafe { imath_sys::MP_OK } {
                return Err(Error::ReadStringTruncated);
            }
        }

        // This is safe bc we produced the char_ptr earlier from a CString
        let _ = unsafe { CString::from_raw(char_ptr) };

        Ok(
            // This cast is safe (from MaybeUninit<imath_sys::mpq_t> to imath_sys::mpq_t)
            // because the value is now initialized.
            unsafe { Rational::from_raw(Box::into_raw(init).cast()) },
        )
    }

    // Reports the minimum number of characters required to represent `z` as a
    // zero-terminated string in base-10.
    #[allow(dead_code)]
    pub(crate) fn required_display_len(&self) -> usize {
        let self_raw = self.as_raw();

        // This is safe bc self_raw has been initialized and 10 is within the range
        // `[MP_MIN_RADIX, MP_MAX_RADIX]`
        let len = unsafe { imath_sys::mp_rat_string_len(self_raw, 10) };

        // The output of the call is an i32, check that it is gte zero.
        assert!(len >= 0);
        len as usize
    }

    pub(crate) fn to_cstring(&self) -> CString {
        let required_len = self.required_display_len();
        let self_raw = self.as_raw();

        let mut char_vec: Vec<u8> = Vec::with_capacity(required_len);
        let res = {
            let char_ptr = char_vec.as_mut_ptr();
            let cap = char_vec.capacity();

            debug_assert_eq!(required_len, cap);
            unsafe {
                imath_sys::mp_rat_to_string(self_raw, 10, char_ptr as *mut _, required_len as i32)
            }
        };

        imath_check_panic!(res, "Writing the value as a string failed!");

        // Setting the length is safe bc we now that the `mp_int_to_string`
        // should have used the entire capacity to write to
        // string.
        unsafe {
            char_vec.set_len(required_len);
        }

        // At this point, char_vec is a zero-terminated vector containing a string
        // representation of the integer.
        let without_nul = &char_vec.as_slice()[..(required_len - 1)];

        CString::new(without_nul).expect("Failed to produce a valid CString")
    }

    #[allow(dead_code)]
    pub(crate) fn set_value(&mut self, numer: impl Into<c_long>, denom: impl Into<c_long>) {
        let self_raw = self.as_raw();

        let res = unsafe { imath_sys::mp_rat_set_value(self_raw, numer.into(), denom.into()) };

        imath_check_panic!(res, "Setting the value failed!");
    }
}

/// Parse a rational value from a string having one of the following formats,
/// each with an optional leading sign flag:
///
/// - `n`, integer format, e.g. "123"
/// - `n/d`, ratio format, e.g., "-12/5"
/// - `z.ffff`, decimal format, e.g., "1.627"
///
/// After successfully parsing, the rational will be reduced by factoring out
/// common multiples of the numerator and denominators, as if `Rational::reduce`
/// was called on the value.
impl FromStr for Rational {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        Rational::from_string_repr(s)
    }
}

impl fmt::Debug for Rational {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // This is safe bc self has been initialized
        let num = self.numerator();
        let den = self.denominator();

        write!(f, "Rational {{ num: {:?}, den: {:?} }}", num, den)
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let string_repr = self.to_cstring();

        f.write_str(string_repr.to_str().unwrap())
    }
}

impl Default for Rational {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Rational {
    fn clone(&self) -> Self {
        Self::copy_init(self)
    }

    fn clone_from(&mut self, source: &Self) {
        source.copy_to(self);
    }
}

impl Drop for Rational {
    fn drop(&mut self) {
        unsafe {
            let raw = self.as_raw();

            // This will ensure that the memory holding the rational data (the digits?) is
            // not leaked.
            imath_sys::mp_rat_clear(raw);

            // This will ensure that the memory that held the `imath_sys::mpq_t` is not
            // leaked.
            drop(Box::from_raw(raw));
        }
    }
}

/// This is the different ways to round a rational number when converting to a
/// decimal format.
pub enum RoundMode {
    /// Truncates the value toward zero.
    Down,
    /// Rounds the value away from zero.
    Up,
    /// Rounds the value to nearest digit, half rounds upward.
    HalfUp,
    /// Rounds the value to nearest digit, half goes toward zero.
    HalfDown,
}

impl Into<imath_sys::mp_round_mode> for RoundMode {
    fn into(self) -> imath_sys::mp_round_mode {
        use RoundMode::*;
        match self {
            Down => imath_sys::mp_round_mode_MP_ROUND_DOWN,
            Up => imath_sys::mp_round_mode_MP_ROUND_UP,
            HalfDown => imath_sys::mp_round_mode_MP_ROUND_HALF_DOWN,
            HalfUp => imath_sys::mp_round_mode_MP_ROUND_HALF_UP,
        }
    }
}

impl TryFrom<imath_sys::mp_round_mode> for RoundMode {
    type Error = Error;

    fn try_from(src: imath_sys::mp_round_mode) -> Result<Self, Error> {
        use RoundMode::*;
        match src {
            imath_sys::mp_round_mode_MP_ROUND_DOWN => Ok(Down),
            imath_sys::mp_round_mode_MP_ROUND_UP => Ok(Up),
            imath_sys::mp_round_mode_MP_ROUND_HALF_DOWN => Ok(HalfDown),
            imath_sys::mp_round_mode_MP_ROUND_HALF_UP => Ok(HalfUp),
            _ => Err(Error::UnknownRoundingMode),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create_default_rational() {
        let rat = Rational::new();

        assert_eq!(rat.numerator(), Integer::from(0));
        assert_eq!(rat.denominator(), Integer::from(1));
    }

    #[test]
    fn parse_normal_rational() {
        let rat: Rational = "12345/67890".parse().unwrap();

        assert_eq!(rat.numerator(), Integer::from(823));
        assert_eq!(rat.denominator(), Integer::from(4526));
    }

    #[test]
    fn parse_integer_rational() {
        let rat: Rational = "12345".parse().unwrap();

        assert_eq!(rat.numerator(), Integer::from(12345));
        assert_eq!(rat.denominator(), Integer::from(1));
    }

    #[test]
    fn parse_decimal_rational() {
        let rat: Rational = "1.2345".parse().unwrap();

        assert_eq!(rat.numerator(), Integer::from(2469));
        assert_eq!(rat.denominator(), Integer::from(2000));
    }

    #[test]
    fn reduce_rational() {
        let mut rat = Rational::new();

        rat.set_value(10, 20);
        rat.reduce();
        assert_eq!(rat.numerator(), Integer::from(1));
        assert_eq!(rat.denominator(), Integer::from(2));

        rat.set_value(0, 1234);
        rat.reduce();
        assert_eq!(rat.numerator(), Integer::from(0));
        assert_eq!(rat.denominator(), Integer::from(1));

        rat.set_value(13412, 13412);
        rat.reduce();
        assert_eq!(rat.numerator(), Integer::from(1));
        assert_eq!(rat.denominator(), Integer::from(1));
    }

    #[test]
    fn clone_rational() {
        let a: Rational =
            "98712698346126837461287318238761234897612839471623487619872364981726348176234"
                .parse()
                .unwrap();

        let b = a.clone();

        assert_eq!(a.numerator(), b.numerator());
        assert_eq!(a.denominator(), b.denominator());
    }
}