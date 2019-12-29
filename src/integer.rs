use crate::error::{Error, Result};
use core::{
    cmp::Ordering,
    fmt,
    mem::{self, MaybeUninit},
    ptr::NonNull,
    str::FromStr,
};
use std::{
    alloc,
    ffi::CString,
    os::raw::{c_long, c_ulong},
};

pub(crate) mod comparison;
pub(crate) mod conversions;
pub(crate) mod ops;

/// Multiple precision integer value. Always heap allocated, not safe for
/// sharing across threads.
#[repr(transparent)]
pub struct Integer {
    // This value must be constructed from a Box and then when Drop, must be reconstructed so that
    // the Box Drop can free the memory used.
    raw: NonNull<imath_sys::mpz_t>,
}

impl Integer {
    pub(crate) fn uninit() -> Box<MaybeUninit<imath_sys::mpz_t>> {
        // Replace with Box::new_uninit when it is stable (1.40 maybe?).
        let layout = alloc::Layout::new::<MaybeUninit<imath_sys::mpz_t>>();
        let ptr = unsafe { alloc::alloc(layout) };
        // This cast is safe bc the layout was specified for
        // MaybeUninit<imath_sys::mpz_t>
        unsafe { Box::from_raw(ptr.cast()) }
    }

    /// Construct a new integer with a default value of zero.
    pub fn new() -> Self {
        Self::from_c_long(0)
    }

    pub(crate) fn copy_init(other: &Self) -> Self {
        let mut init = Integer::uninit();
        let other_raw = other.raw.as_ptr();

        {
            // This is safe bc init is entirely local. raw_mpz is also scoped to be less
            // than the lifetime of the value init
            let raw_mpz = init.as_mut_ptr();

            // This is safe bc a valid structure is provided to the unsafe methods. And the
            // src value is of the correct type?
            let res = unsafe { imath_sys::mp_int_init_copy(raw_mpz, other_raw) };

            // Accessing this is safe bc the MP_OK value is only ever used as an error
            // condition.
            if res != unsafe { imath_sys::MP_OK } {
                panic!("Value init failed! {:?}", res);
            }
        }

        // This cast is safe (from MaybeUninit<imath_sys::mpz_t> to imath_sys::mpz_t)
        // because the value is now initialized.
        unsafe { Integer::from_raw(Box::into_raw(init).cast()) }
    }

    /// Construct an Integer from a raw non-null pointer to `imath_sys::mpz_t`.
    ///
    /// # Safety
    ///
    /// This function must only every be called once for a given pointer, and
    /// the pointer must point to an initialized `imath_sys::mpz_t` struct. The
    /// recommendation is to only use raw pointers from the `Integer::into_raw`
    /// function.
    ///
    /// In ths context, initialized means that the `imath_sys::mpz_t` has been
    /// the argument of a call to `imath_sys::mp_int_init`.
    pub unsafe fn from_raw(raw: *mut imath_sys::mpz_t) -> Self {
        assert!(!raw.is_null());

        // This is safe bc the invariants of the function and because it was checked
        // that the pointer is not null.
        #[allow(unused_unsafe)]
        let raw = unsafe { NonNull::new_unchecked(raw) };

        Integer { raw }
    }

    /// Consumes the Integer, returning a wrapped raw pointer.
    pub fn into_raw(mut integer: Integer) -> *mut imath_sys::mpz_t {
        let raw = mem::replace(&mut integer.raw, NonNull::dangling());

        // The destructor does not need to run, as we are intentionally leaking the
        // resources here.
        mem::forget(integer);

        raw.as_ptr()
    }

    pub(crate) fn from_c_long(src: impl Into<c_long>) -> Self {
        let mut init = Integer::uninit();

        {
            // This is safe bc init is entirely local. raw_mpz is also scoped to be less
            // than the lifetime of the value init
            let raw_mpz = init.as_mut_ptr();

            // This is safe bc a valid structure is provided to the unsafe methods. And the
            // src value is of the correct type?
            let res = unsafe { imath_sys::mp_int_init_value(raw_mpz, src.into()) };

            // Accessing this is safe bc the MP_OK value is only ever used as an error
            // condition.
            if res != unsafe { imath_sys::MP_OK } {
                panic!("Value init failed! {:?}", res);
            }
        }

        // This cast is safe (from MaybeUninit<imath_sys::mpz_t> to imath_sys::mpz_t)
        // because the value is now initialized.
        unsafe { Integer::from_raw(Box::into_raw(init).cast()) }
    }

    pub(crate) fn from_string_repr(src: impl ToString) -> Result<Self> {
        let string_repr =
            CString::new(src.to_string()).map_err(|_| Error::StringReprContainedNul)?;
        let char_ptr = string_repr.into_raw();

        let mut init = Integer::uninit();

        {
            // This is safe bc init is entirely local. raw_mpz is also scoped to be less
            // than the lifetime of the value init
            let raw_mpz = init.as_mut_ptr();

            // This is safe bc a valid structure is provided to the unsafe methods. And the
            // src value is of the correct type?
            let res_init = unsafe { imath_sys::mp_int_init(raw_mpz) };

            // Accessing this is safe bc the MP_OK value is only ever used as an error
            // condition.
            if res_init != unsafe { imath_sys::MP_OK } {
                panic!("Init failed! {:?}", res_init);
            }

            // This is safe bc all the data provided to the function is correctly setup
            // (integer was allocated/initialized, char_ptr is 0-terminated).
            let res_read = unsafe { imath_sys::mp_int_read_string(raw_mpz, 10, char_ptr) };

            // Accessing this is safe bc the MP_OK value is only ever used as an error
            // condition.
            if res_read != unsafe { imath_sys::MP_OK } {
                return Err(Error::ReadStringTruncated);
            }
        }

        // This is safe bc we produced the char_ptr earlier from a CString
        let _ = unsafe { CString::from_raw(char_ptr) };

        Ok(
            // This `Integer::from_raw` is safe because
            //
            // This cast is safe (from MaybeUninit<imath_sys::mpz_t> to imath_sys::mpz_t)
            // because the value is now initialized.
            unsafe { Integer::from_raw(Box::into_raw(init).cast()) },
        )
    }

    // Reports the minimum number of characters required to represent `z` as a
    // zero-terminated string in base-10.
    pub(crate) fn required_display_len(&self) -> usize {
        let self_raw = self.raw.as_ptr();

        // This is safe bc self has been initialized
        let len = unsafe { imath_sys::mp_int_string_len(self_raw, 10) };

        // The output of the call is an i32, check that it is gte zero.
        assert!(len >= 0);
        len as usize
    }

    pub(crate) fn to_cstring(&self) -> CString {
        let required_len = self.required_display_len();
        let self_raw = self.raw.as_ptr();

        let mut char_vec: Vec<u8> = Vec::with_capacity(required_len);
        let res = {
            let char_ptr = char_vec.as_mut_ptr();
            let cap = char_vec.capacity();

            debug_assert_eq!(required_len, cap);
            unsafe {
                imath_sys::mp_int_to_string(self_raw, 10, char_ptr as *mut _, required_len as i32)
            }
        };

        // Accessing this is safe bc the MP_OK value is only ever used as an error
        // condition.
        if res != unsafe { imath_sys::MP_OK } {
            panic!("Writing the value as a string failed! {:?}", res);
        }

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

    /// Replaces the value of `other` with a copy of the value of `self`. No new
    /// memory is allocated unless `self` has more significant digits than
    /// `other` has allocated.
    pub fn copy_to(&self, other: &mut Self) {
        let self_raw = self.raw.as_ptr();
        let other_raw = other.raw.as_ptr();

        // This is safe bc self has been initialized with a value
        let res = unsafe { imath_sys::mp_int_copy(other_raw, self_raw) };

        if res != unsafe { imath_sys::MP_OK } {
            panic!("Copying the value failed! {:?}", res);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_value(&mut self, value: impl Into<c_long>) {
        let self_raw = self.raw.as_ptr();

        let res = unsafe { imath_sys::mp_int_set_value(self_raw, value.into()) };

        if res != unsafe { imath_sys::MP_OK } {
            panic!("Setting the value failed! {:?}", res);
        }
    }

    /// Set value of integer to zero
    pub fn zero(&mut self) {
        let self_raw = self.raw.as_ptr();

        unsafe { imath_sys::mp_int_zero(self_raw) };
    }

    /// Compare two integers
    pub fn compare(&self, rhs: &Self) -> Ordering {
        let self_raw = self.raw.as_ptr();
        let rhs_raw = rhs.raw.as_ptr();

        // This is safe bc both self & rhs have been initialized correctly
        let raw_cmp = unsafe { imath_sys::mp_int_compare(self_raw, rhs_raw) };

        raw_cmp.cmp(&0)
    }

    /// Compare the magnitude of two integers, not taking sign into account.
    pub fn compare_magnitude(&self, rhs: &Self) -> Ordering {
        let self_raw = self.raw.as_ptr();
        let rhs_raw = rhs.raw.as_ptr();

        // This is safe bc both self & rhs have been initialized correctly
        let raw_cmp = unsafe { imath_sys::mp_int_compare_unsigned(self_raw, rhs_raw) };

        raw_cmp.cmp(&0)
    }

    /// Compare an integer to zero.
    pub fn compare_zero(&self) -> Ordering {
        let self_raw = self.raw.as_ptr();

        // This is safe bc both self has been initialized correctly
        let raw_cmp = unsafe { imath_sys::mp_int_compare_zero(self_raw) };

        raw_cmp.cmp(&0)
    }

    pub(crate) fn compare_c_long(&self, value: impl Into<c_long>) -> Ordering {
        let self_raw = self.raw.as_ptr();
        let value = value.into();

        // This is safe bc both self has been initialized correctly
        let raw_cmp = unsafe { imath_sys::mp_int_compare_value(self_raw, value) };

        raw_cmp.cmp(&0)
    }

    #[allow(dead_code)]
    pub(crate) fn compare_c_ulong(&self, value: impl Into<c_ulong>) -> Ordering {
        let self_raw = self.raw.as_ptr();
        let value = value.into();

        // This is safe bc both self has been initialized correctly
        let raw_cmp = unsafe { imath_sys::mp_int_compare_uvalue(self_raw, value) };

        raw_cmp.cmp(&0)
    }

    pub(crate) fn try_into_c_long(&self) -> Result<c_long> {
        let self_raw = self.raw.as_ptr();
        let mut out: c_long = 0;
        let out_raw = (&mut out) as *mut _;

        let res = unsafe { imath_sys::mp_int_to_int(self_raw, out_raw) };

        if res == unsafe { imath_sys::MP_OK } {
            Ok(out)
        } else {
            Err(Error::ConversionOutsideRange)
        }
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let string_repr = self.to_cstring();

        f.write_str(string_repr.to_str().unwrap())
    }
}

impl fmt::Debug for Integer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // This is safe bc self has been initialized
        let imath_sys::mpz_t {
            single,
            digits,
            alloc,
            used,
            sign,
        } = unsafe { *self.raw.as_ptr() };

        write!(
            f,
            "Integer {{ single: {:?}, digits: {:p}, alloc: {:?}, used: {:?}, sign: {:?} }}",
            single, digits, alloc, used, sign
        )
    }
}

impl Clone for Integer {
    fn clone(&self) -> Self {
        Integer::copy_init(self)
    }

    fn clone_from(&mut self, source: &Self) {
        source.copy_to(self);
    }
}

impl Default for Integer {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for Integer {
    type Err = Error;

    fn from_str(s: &str) -> core::result::Result<Self, Self::Err> {
        Integer::from_string_repr(s)
    }
}

impl Drop for Integer {
    fn drop(&mut self) {
        unsafe {
            let raw = self.raw.as_ptr();

            // This will ensure that the memory holding the integer data (the digits?) is
            // not leaked.
            imath_sys::mp_int_clear(raw);

            // This will ensure that the memory that held the `imath_sys::mpz_t` is not
            // leaked.
            drop(Box::from_raw(raw));
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create_default_integer() {
        let int = Integer::new();

        let string_repr = int.to_string();
        assert_eq!(&string_repr, "0");
    }

    #[test]
    fn create_integer_with_value() {
        let int = Integer::from_c_long(20000);

        let string_repr = int.to_string();
        assert_eq!(&string_repr, "20000");
    }

    #[test]
    fn parse_big_integer() {
        let int: Integer =
            "98712698346126837461287318238761234897612839471623487619872364981726348176234"
                .parse()
                .unwrap();
        #[allow(clippy::eq_op)]
        let zero = &int - &int;

        assert_eq!(zero, 0)
    }

    #[test]
    fn zero_integer() {
        let mut big_int: Integer =
            "98712698346126837461287318238761234897612839471623487619872364981726348176234"
                .parse()
                .unwrap();
        let mut small_int: Integer = (-4_565_234).into();

        big_int.zero();
        assert_eq!(big_int, 0);

        small_int.zero();
        assert_eq!(small_int, 0);
    }

    #[test]
    fn clone_integer() {
        let a: Integer =
            "98712698346126837461287318238761234897612839471623487619872364981726348176234"
                .parse()
                .unwrap();

        let b = a.clone();

        assert_eq!(a, b);
    }
}
