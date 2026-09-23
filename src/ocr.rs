//! Minimal, persistent Tesseract C API binding. The handle never leaves its
//! worker thread; all returned C strings are released with TessDeleteText.
use crate::model::{Frame, normalize};
use anyhow::{Context, Result};
use libloading::Library;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    time::Instant,
};

type Handle = *mut c_void;
pub struct Ocr {
    _library: Library,
    handle: Handle,
    delete: unsafe extern "C" fn(Handle),
    set_image: unsafe extern "C" fn(Handle, *const u8, c_int, c_int, c_int, c_int),
    get_text: unsafe extern "C" fn(Handle) -> *mut c_char,
    delete_text: unsafe extern "C" fn(*mut c_char),
    confidence: unsafe extern "C" fn(Handle) -> c_int,
    clear_adaptive: unsafe extern "C" fn(Handle),
}

#[derive(Debug)]
pub struct OcrOutput {
    pub text: String,
    pub confidence: i32,
    pub elapsed_ms: u64,
}

impl Ocr {
    pub fn new() -> Result<Self> {
        // SAFETY: symbols have the signatures in tesseract/capi.h. Library
        // remains owned by Self until after the API handle has been deleted.
        unsafe {
            let library = Library::new("libtesseract.so.5")
                .context("Instale libtesseract5 e tesseract-ocr-eng.")?;
            let create = library.get::<unsafe extern "C" fn() -> Handle>(b"TessBaseAPICreate\0")?;
            let delete = *library.get(b"TessBaseAPIDelete\0")?;
            let init =
                library
                    .get::<unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int>(
                        b"TessBaseAPIInit3\0",
                    )?;
            let set_image = *library.get(b"TessBaseAPISetImage\0")?;
            let get_text = *library.get(b"TessBaseAPIGetUTF8Text\0")?;
            let delete_text = *library.get(b"TessDeleteText\0")?;
            let confidence = *library.get(b"TessBaseAPIMeanTextConf\0")?;
            let clear_adaptive = *library.get(b"TessBaseAPIClearAdaptiveClassifier\0")?;
            let set_mode = library
                .get::<unsafe extern "C" fn(Handle, c_int)>(b"TessBaseAPISetPageSegMode\0")?;
            let handle = create();
            anyhow::ensure!(!handle.is_null(), "Não foi possível criar o OCR.");
            let path = std::env::var("TESSDATA_PREFIX")
                .ok()
                .map(CString::new)
                .transpose()?;
            if init(
                handle,
                path.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                c"eng".as_ptr(),
            ) != 0
            {
                let delete: unsafe extern "C" fn(Handle) = delete;
                delete(handle);
                anyhow::bail!(
                    "Modelo inglês indisponível. Instale tesseract-ocr-eng ou configure TESSDATA_PREFIX."
                );
            }
            set_mode(handle, 6); // A single uniform text block, ideal for a selected dialog.
            Ok(Self {
                _library: library,
                handle,
                delete,
                set_image,
                get_text,
                delete_text,
                confidence,
                clear_adaptive,
            })
        }
    }

    pub fn recognize(&mut self, frame: &Frame) -> Result<OcrOutput> {
        anyhow::ensure!(
            frame.gray.len() == frame.width as usize * frame.height as usize
                && !frame.gray.is_empty(),
            "Quadro OCR inválido."
        );
        let start = Instant::now();
        let (pixels, width, height) = preprocess(frame);
        // SAFETY: pixels is contiguous 8-bit grayscale and remains alive until
        // synchronous recognition completes; only this thread uses the handle.
        unsafe {
            (self.clear_adaptive)(self.handle);
            (self.set_image)(
                self.handle,
                pixels.as_ptr(),
                width as i32,
                height as i32,
                1,
                width as i32,
            );
            let text_ptr = (self.get_text)(self.handle);
            anyhow::ensure!(!text_ptr.is_null(), "OCR não retornou um resultado.");
            let text = CStr::from_ptr(text_ptr).to_string_lossy().into_owned();
            (self.delete_text)(text_ptr);
            let confidence = (self.confidence)(self.handle);
            Ok(OcrOutput {
                text: if confidence >= 40 {
                    normalize(&text)
                } else {
                    String::new()
                },
                confidence,
                elapsed_ms: start.elapsed().as_millis() as u64,
            })
        }
    }
}

impl Drop for Ocr {
    fn drop(&mut self) {
        unsafe {
            (self.delete)(self.handle);
        }
    }
}

fn preprocess(frame: &Frame) -> (Vec<u8>, u32, u32) {
    let scale = if frame.width <= 1200 && frame.height <= 400 {
        2
    } else {
        1
    };
    let width = frame.width * scale;
    let height = frame.height * scale;
    let mut histogram = [0u32; 256];
    for &pixel in &frame.gray {
        histogram[pixel as usize] += 1;
    }
    let background = histogram
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| *n)
        .map(|(i, _)| i)
        .unwrap_or(255);
    let low = *frame.gray.iter().min().unwrap_or(&0) as i32;
    let high = *frame.gray.iter().max().unwrap_or(&255) as i32;
    let range = (high - low).max(32);
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let p = frame.gray[((y / scale) * frame.width + x / scale) as usize] as i32;
            let value = ((p - low) * 255 / range).clamp(0, 255) as u8;
            pixels.push(if background < 128 { 255 - value } else { value });
        }
    }
    (pixels, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dark_dialogue_becomes_light_background() {
        let frame = Frame {
            width: 4,
            height: 2,
            gray: vec![0, 0, 255, 0, 0, 0, 255, 0],
            captured: Instant::now(),
        };
        let (data, width, height) = preprocess(&frame);
        assert_eq!((width, height), (8, 4));
        assert_eq!(data[0], 255);
        assert_eq!(data[4], 0);
    }
}
