#![cfg(feature = "aeron-ffi")]

use libc::{c_char, c_int, c_longlong, c_void};
use std::ffi::CString;
use std::ptr::null_mut;
use std::thread::sleep;
use std::time::{Duration, Instant};

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
    #[allow(unused)]
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
    fn aeron_subscription_poll(
        subscription: *mut aeron_subscription_t,
        handler: Option<extern "C" fn(*mut c_void, *const c_void, c_longlong, *const c_void)>,
        clientd: *mut c_void,
        fragment_limit: c_int,
    ) -> c_int;
}

pub struct AeronClient {
    pub(crate) ctx: *mut aeron_context_t,
    pub(crate) aeron: *mut aeron_t,
}

unsafe impl Send for AeronClient {}
unsafe impl Sync for AeronClient {}

impl AeronClient {
    pub fn connect() -> std::io::Result<Self> {
        unsafe {
            let mut ctx: *mut aeron_context_t = null_mut();
            if aeron_context_init(&mut ctx) != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "aeron_context_init failed",
                ));
            }
            let mut aeron: *mut aeron_t = null_mut();
            if aeron_init(&mut aeron, ctx) != 0 {
                let _ = aeron_context_close(ctx);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "aeron_init failed",
                ));
            }
            if aeron_start(aeron) != 0 {
                let _ = aeron_close(aeron);
                let _ = aeron_context_close(ctx);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "aeron_start failed",
                ));
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
            let r =
                aeron_publication_add(aeron.aeron, &mut pub_ptr, c.as_ptr(), stream_id as c_int);
            if r < 0 || pub_ptr.is_null() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "aeron_publication_add failed",
                ));
            }
            Ok(Self { pub_ptr })
        }
    }

    pub fn offer(&self, data: &[u8]) -> std::io::Result<i64> {
        unsafe {
            let res = aeron_publication_offer(
                self.pub_ptr,
                data.as_ptr() as *const c_void,
                data.len() as c_longlong,
                null_mut(),
                null_mut(),
            );
            if res < 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("offer failed: {}", res),
                ));
            }
            Ok(res as i64)
        }
    }

    pub fn offer_retry(
        &self,
        data: &[u8],
        max_retries: usize,
        backoff_ms: u64,
        _fragment_limit: i32,
    ) -> std::io::Result<i64> {
        let mut tries = 0;
        loop {
            match self.offer(data) {
                Ok(n) => return Ok(n),
                Err(e) => {
                    tries += 1;
                    if tries >= max_retries {
                        return Err(e);
                    }
                    sleep(Duration::from_millis(backoff_ms));
                }
            }
        }
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        unsafe {
            let _ = aeron_publication_close(self.pub_ptr);
        }
    }
}

pub struct Subscription {
    sub_ptr: *mut aeron_subscription_t,
}

unsafe impl Send for Subscription {}
unsafe impl Sync for Subscription {}

impl Subscription {
    pub fn add(aeron: &AeronClient, channel: &str, stream_id: i32) -> std::io::Result<Self> {
        unsafe {
            let c = CString::new(channel).unwrap();
            let mut sub_ptr: *mut aeron_subscription_t = null_mut();
            let r = aeron_subscription_add(
                aeron.aeron,
                &mut sub_ptr,
                c.as_ptr(),
                stream_id as c_int,
                None,
                null_mut(),
                null_mut(),
                null_mut(),
            );
            if r < 0 || sub_ptr.is_null() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "aeron_subscription_add failed",
                ));
            }
            Ok(Self { sub_ptr })
        }
    }

    pub fn poll_collect(&self, max_ms: u64, fragment_limit: i32) -> Vec<Vec<u8>> {
        extern "C" fn handler(
            clientd: *mut c_void,
            buffer: *const c_void,
            length: c_longlong,
            _header: *const c_void,
        ) {
            unsafe {
                let col = &mut *(clientd as *mut Collector);
                if length > 0 {
                    let slice = std::slice::from_raw_parts(buffer as *const u8, length as usize);
                    col.fragments.push(slice.to_vec());
                    col.count += 1;
                }
            }
        }
        struct Collector {
            fragments: Vec<Vec<u8>>,
            count: usize,
        }
        let mut col = Collector {
            fragments: Vec::new(),
            count: 0,
        };
        let start = Instant::now();
        unsafe {
            while start.elapsed() < Duration::from_millis(max_ms) {
                let polled = aeron_subscription_poll(
                    self.sub_ptr,
                    Some(handler),
                    &mut col as *mut _ as *mut c_void,
                    fragment_limit as c_int,
                );
                if polled < 0 {
                    break;
                }
                if polled == 0 {
                    std::thread::yield_now();
                }
            }
        }
        col.fragments
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        unsafe {
            let _ = aeron_subscription_close(self.sub_ptr);
        }
    }
}
