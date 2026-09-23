//! Explicit opt-in live test: one synthetic crop to Vision, then its text to Translation.
//! Credentials are read from Secret Service into memory, never printed or passed in arguments.
use area_translator::{
    model::{Frame, normalize},
    ocr::Ocr,
    translate::Translator,
};
use std::{io::BufReader, process::Command, time::Instant};

#[tokio::test]
#[ignore = "requires explicit AREA_TRANSLATOR_CLOUD_TEST=1, configured keyring and billable Google APIs"]
async fn synthetic_crop_through_vision_and_translation() {
    assert_eq!(
        std::env::var("AREA_TRANSLATOR_CLOUD_TEST").as_deref(),
        Ok("1")
    );
    let output = Command::new("gjs")
        .args([
            "-c",
            r#"
const Secret = imports.gi.Secret;
const schema = new Secret.Schema('io.github.areatranslator.Credential', Secret.SchemaFlags.NONE,
    {application: Secret.SchemaAttributeType.STRING});
const key = Secret.password_lookup_sync(schema, {application: 'area-translator'}, null);
if (!key) throw new Error('Configure the credential in the application first.');
print(key);
"#,
        ])
        .output()
        .expect("Could not access desktop keyring");
    assert!(
        output.status.success(),
        "Could not read configured key; details withheld"
    );
    let key = String::from_utf8(output.stdout).expect("Invalid credential encoding");
    let key = key.trim();
    assert!(!key.is_empty());
    let mut reader = png::Decoder::new(BufReader::new(
        std::fs::File::open("tests/fixtures/dialog.png").unwrap(),
    ))
    .read_info()
    .unwrap();
    let mut gray = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut gray).unwrap();
    gray.truncate(info.buffer_size());
    let frame = Frame {
        width: info.width,
        height: info.height,
        gray,
        captured: Instant::now(),
    };
    let output = Ocr::new()
        .unwrap()
        .recognize(key, frame)
        .await
        .expect("Cloud Vision request failed");
    let expected = std::fs::read_to_string("tests/fixtures/dialog.txt").unwrap();
    assert!(
        normalize(&output.text).eq_ignore_ascii_case(&normalize(&expected)),
        "Vision differed from the synthetic fixture text"
    );
    let start = Instant::now();
    let translated = Translator::new()
        .unwrap()
        .translate(key, &normalize(&output.text))
        .await
        .expect("Translation request failed");
    assert!(!translated.is_empty());
    println!(
        "Cloud Vision: {} characters, {} ms total / {} ms API; Translation: {} characters, {} ms. No image or credential saved.",
        output.text.chars().count(),
        output.elapsed_ms,
        output.api_ms,
        translated.chars().count(),
        start.elapsed().as_millis()
    );
}
