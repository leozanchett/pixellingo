//! Run explicitly with installed native dependencies:
//! scripts/dev.sh cargo test --test ocr_native -- --ignored --nocapture
use area_translator::{model::Frame, ocr::Ocr};
use std::{fs::File, io::BufReader, time::Instant};

#[test]
#[ignore = "requires libtesseract5 and the English fast model"]
fn real_ocr_on_synthetic_dialogues() {
    let mut ocr = Ocr::new().unwrap();
    for name in [
        "dialog",
        "dark",
        "small",
        "pixelated",
        "multiline",
        "blank",
        "scenery",
        "sparse-dialog",
        "sparse-dialog-moved",
        "sparse-empty",
        "dialog",
    ] {
        let decoder = png::Decoder::new(BufReader::new(
            File::open(format!("tests/fixtures/{name}.png")).unwrap(),
        ));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        pixels.truncate(info.buffer_size());
        assert_eq!(info.color_type, png::ColorType::Grayscale);
        let frame = Frame {
            width: info.width,
            height: info.height,
            gray: pixels,
            captured: Instant::now(),
        };
        let output = ocr.recognize(&frame).unwrap();
        let expected = std::fs::read_to_string(format!("tests/fixtures/{name}.txt")).unwrap();
        println!(
            "{name}: confidence={}, time={}ms, text={:?}",
            output.confidence, output.elapsed_ms, output.text
        );
        if name == "pixelated" {
            // Pixel fonts can merge word spacing. Require every letter and
            // punctuation mark, while reporting the original OCR above.
            assert_eq!(
                output.text.replace(' ', ""),
                expected.trim().replace(' ', ""),
                "fixture {name}"
            );
        } else {
            assert_eq!(output.text, expected.trim(), "fixture {name}");
        }
    }
}
