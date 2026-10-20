//! Moving pixels between Spooch and the X server, through MIT-SHM when the
//! server offers it and plain GetImage/PutImage otherwise.
//!
//! The frozen desktop itself never leaves the server (see `mod.rs`); only
//! the selected region comes over, once when editing starts, and goes back
//! each time the marks change. With MIT-SHM those copies go through memory
//! shared with the server instead of the socket.

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, Drawable, Gcontext, ImageFormat};
use x11rb::rust_connection::RustConnection;

/// A System V shared memory segment attached on both sides.
pub struct Shm {
    seg: u32,
    ptr: *mut u8,
    size: usize,
    /// The server may still be reading the segment for a put we sent.
    in_flight: bool,
}

impl Shm {
    pub fn new(conn: &RustConnection, size: usize) -> Result<Self, String> {
        conn.extension_information(x11rb::protocol::shm::X11_EXTENSION_NAME)
            .map_err(|e| e.to_string())?
            .ok_or("server has no MIT-SHM")?;
        // Before shmget, so no early return can strand a segment.
        let seg = conn.generate_id().map_err(|e| e.to_string())?;
        // SAFETY: plain SysV calls; the segment is marked IPC_RMID before
        // returning, and `ptr` stays attached until `Drop`.
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if id < 0 {
                return Err(format!("shmget: {}", std::io::Error::last_os_error()));
            }
            let ptr = libc::shmat(id, std::ptr::null(), 0);
            if ptr as isize == -1 {
                libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                return Err(format!("shmat: {}", std::io::Error::last_os_error()));
            }
            let attached = conn
                .shm_attach(seg, id as u32, false)
                .map_err(|e| e.to_string())
                .and_then(|c| c.check().map_err(|e| e.to_string()));
            // Marked for removal now; it goes away once both sides detach.
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            if let Err(e) = attached {
                libc::shmdt(ptr);
                return Err(format!("shm_attach (remote X server?): {e}"));
            }
            Ok(Self {
                seg,
                ptr: ptr.cast(),
                size,
                in_flight: false,
            })
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// The segment's bytes, after waiting for the server to finish any put
    /// that still reads them.
    fn bytes_mut(&mut self, conn: &RustConnection) -> Result<&mut [u8], String> {
        if self.in_flight {
            conn.get_input_focus()
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?;
            self.in_flight = false;
        }
        // SAFETY: `ptr` is attached for `size` bytes until `Drop`, and the
        // server no longer reads it (`in_flight` is clear).
        Ok(unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) })
    }

    /// Detaches the server's side too. Without this the segment, already
    /// marked for removal, stays alive in the X server until we disconnect.
    fn detach(self, conn: &RustConnection) {
        let _ = conn.shm_detach(self.seg);
    }
}

impl Drop for Shm {
    fn drop(&mut self) {
        unsafe {
            libc::shmdt(self.ptr.cast());
        }
    }
}

/// Reads and writes rectangles of pixels. The shared segment is created on
/// first use, grown as needed, and released between captures so an idle
/// Spooch holds no pixel memory.
pub struct Transfer {
    use_shm: bool,
    shm: Option<Shm>,
}

impl Transfer {
    pub fn new(use_shm: bool) -> Self {
        Self { use_shm, shm: None }
    }

    pub fn name(&self) -> &'static str {
        if self.use_shm { "mit-shm" } else { "getimage" }
    }

    pub fn release(&mut self, conn: &RustConnection) {
        if let Some(shm) = self.shm.take() {
            shm.detach(conn);
        }
    }

    fn shm(&mut self, conn: &RustConnection, len: usize) -> Option<&mut Shm> {
        if !self.use_shm {
            return None;
        }
        if self.shm.as_ref().is_none_or(|s| s.size() < len) {
            self.release(conn);
            match Shm::new(conn, len) {
                Ok(shm) => self.shm = Some(shm),
                Err(e) => {
                    super::log::line(format!("MIT-SHM unavailable ({e}); using GetImage"));
                    self.use_shm = false;
                    return None;
                }
            }
        }
        self.shm.as_mut()
    }

    /// Tightly packed BGRX pixels of a rectangle of `src`.
    pub fn read(
        &mut self,
        conn: &RustConnection,
        src: Drawable,
        (x, y): (i16, i16),
        (w, h): (u16, u16),
    ) -> Result<Vec<u8>, String> {
        let len = w as usize * h as usize * 4;
        let data = match self.shm(conn, len) {
            Some(shm) => {
                shm.bytes_mut(conn)?;
                conn.shm_get_image(
                    src,
                    x,
                    y,
                    w,
                    h,
                    !0,
                    ImageFormat::Z_PIXMAP.into(),
                    shm.seg,
                    0,
                )
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| format!("ShmGetImage: {e}"))?;
                shm.bytes_mut(conn)?[..len].to_vec()
            }
            None => {
                conn.get_image(ImageFormat::Z_PIXMAP, src, x, y, w, h, !0)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| format!("GetImage: {e}"))?
                    .data
            }
        };
        if data.len() < len {
            return Err(format!("read {} bytes, expected {len}", data.len()));
        }
        Ok(data)
    }

    /// Draw tightly packed BGRX pixels onto `dst` at (x, y). `fill` writes
    /// the pixels into the buffer it is given, which saves a copy when the
    /// buffer is the shared segment.
    #[allow(clippy::too_many_arguments)]
    pub fn put(
        &mut self,
        conn: &RustConnection,
        dst: Drawable,
        gc: Gcontext,
        depth: u8,
        (x, y): (i16, i16),
        (w, h): (u16, u16),
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<(), String> {
        let len = w as usize * h as usize * 4;
        if len == 0 {
            return Ok(());
        }
        match self.shm(conn, len) {
            Some(shm) => {
                fill(&mut shm.bytes_mut(conn)?[..len]);
                conn.shm_put_image(
                    dst,
                    gc,
                    w,
                    h,
                    0,
                    0,
                    w,
                    h,
                    x,
                    y,
                    depth,
                    ImageFormat::Z_PIXMAP.into(),
                    false,
                    shm.seg,
                    0,
                )
                .map_err(|e| e.to_string())?;
                shm.in_flight = true;
            }
            None => {
                let mut buf = vec![0u8; len];
                fill(&mut buf);
                // Stay under the request size limit by sending row strips.
                let row = w as usize * 4;
                let max = conn.maximum_request_bytes().saturating_sub(64);
                let rows_per = (max / row).clamp(1, h as usize);
                for (i, strip) in buf.chunks(rows_per * row).enumerate() {
                    let rows = (strip.len() / row) as u16;
                    conn.put_image(
                        ImageFormat::Z_PIXMAP,
                        dst,
                        gc,
                        w,
                        rows,
                        x,
                        y + (i * rows_per) as i16,
                        0,
                        depth,
                        strip,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }
}
