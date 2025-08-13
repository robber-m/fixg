#![cfg(feature = "aeron-ffi")]

use libc::{c_char, c_int, c_longlong, c_void};
use std::ffi::CString;
use std::ptr::null_mut;

#[allow(non_camel_case_types)]
pub type aeron_context_t = c_void;
#[allow(non_camel_case_types)]
pub type aeron_t = c_void;
#[allow(non_camel_case_types)]
pub type aeron_publication_t = c_void;
#[allow(non_camel_case_types)]
pub type aeron_subscription_t = c_void;

#[link(name = "aeron")]
extern "C" {
    fn aeron_context_init(context: *mut *mut aeron_context_t) -> c_int;
    fn aeron_context_close(context: *mut aeron_context_t) -> c_int;

    fn aeron_init(aeron: *mut *mut aeron_t, context: *mut aeron_context_t) -> c_int;
    fn aeron_start(aeron: *mut aeron_t) -> c_int;
    fn aeron_close(aeron: *mut aeron_t) -> c_int;

    fn aeron_publication_add(
        aeron: *mut aeron_t,
        publication: *mut *mut aeron_publication_t,
        channel: *const c_char,
        stream_id: c_int,
    ) -> c_longlong;
    fn aeron_publication_offer(
        publication: *mut aeron_publication_t,
        buffer: *const c_void,
        length: c_longlong,
        reserved_value_supplier: *const c_void,
        reserved_value_supplier_clientd: *mut c_void,
    ) -> c_longlong;
    fn aeron_publication_is_closed(publication: *mut aeron_publication_t) -> c_int;
    fn aeron_publication_close(publication: *mut aeron_publication_t) -> c_int;

    fn aeron_subscription_add(
        aeron: *mut aeron_t,
        subscription: *mut *mut aeron_subscription_t,
        channel: *const c_char,
        stream_id: c_int,
        handler: Option<extern "C" fn(*mut c_void, *const c_void, c_longlong, *const c_void)>,
        clientd: *mut c_void,
        on_available_image: *const c_void,
        on_unavailable_image: *const c_void,
    ) -> c_longlong;
    fn aeron_subscription_close(subscription: *mut aeron_subscription_t) -> c_int;
}

pub struct AeronClient {
    ctx: *mut aeron_context_t,
    aeron: *mut aeron_t,
}

unsafe impl Send for AeronClient {}
unsafe impl Sync for AeronClient {}

impl AeronClient {
    pub fn connect() -> std::io::Result<Self> {
        unsafe {
            let mut ctx: *mut aeron_context_t = null_mut();
            if aeron_context_init(&mut ctx) != 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "aeron_context_init failed"));
            }
            let mut aeron: *mut aeron_t = null_mut();
            if aeron_init(&mut aeron, ctx) != 0 {
                let _ = aeron_context_close(ctx);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "aeron_init failed"));
            }
            if aeron_start(aeron) != 0 {
                let _ = aeron_close(aeron);
                let _ = aeron_context_close(ctx);
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "aeron_start failed"));
            }
            Ok(Self { ctx, aeron })
        }
    }
}

impl Drop for AeronClient {
    fn drop(&mut self) {
        unsafe {
            let _ = aeron_close(self.aeron);
            let _ = aeron_context_close(self.ctx);
        }
    }
}

pub struct Publication {
    pub_ptr: *mut aeron_publication_t,
}

unsafe impl Send for Publication {}
unsafe impl Sync for Publication {}

impl Publication {
    pub fn add(aeron: &AeronClient, channel: &str, stream_id: i32) -> std::io::Result<Self> {
        unsafe {
            let c = CString::new(channel).unwrap();
            let mut pub_ptr: *mut aeron_publication_t = null_mut();
            let r = aeron_publication_add(aeron.aeron, &mut pub_ptr, c.as_ptr(), stream_id as c_int);
            if r < 0 || pub_ptr.is_null() {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "aeron_publication_add failed"));
            }
            Ok(Self { pub_ptr })
        }
    }

    pub fn offer(&self, data: &[u8]) -> std::io::Result<i64> {
        unsafe {
            let res = aeron_publication_offer(self.pub_ptr, data.as_ptr() as *const c_void, data.len() as c_longlong, null_mut(), null_mut());
            if res < 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("offer failed: {}", res)));
            }
            Ok(res as i64)
        }
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        unsafe { let _ = aeron_publication_close(self.pub_ptr); }
    }
}