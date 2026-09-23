use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const SCAN_INTERVAL: Duration = Duration::from_millis(200);
pub const OCR_INTERVAL: Duration = Duration::from_millis(500);
pub const STABLE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn validate(self, width: u32, height: u32) -> anyhow::Result<Self> {
        anyhow::ensure!(
            self.width >= 16 && self.height >= 16,
            "Selecione uma área de pelo menos 16 × 16 pixels."
        );
        anyhow::ensure!(
            self.x.checked_add(self.width).is_some_and(|r| r <= width)
                && self.y.checked_add(self.height).is_some_and(|b| b <= height),
            "Área fora da captura."
        );
        anyhow::ensure!(
            self.width as u64 * self.height as u64 <= 4_000_000,
            "Área muito grande: selecione somente a caixa de texto (máximo 4 megapixels)."
        );
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub gray: Vec<u8>,
    pub captured: Instant,
}

impl Frame {
    /// Small luminance map; spatial changes count, not just average brightness.
    pub fn fingerprint(&self) -> Vec<u8> {
        let w = self.width.min(128) as usize;
        let h = self.height.min(64) as usize;
        let mut result = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                result.push(
                    self.gray[(y * self.height as usize / h) * self.width as usize
                        + x * self.width as usize / w],
                );
            }
        }
        result
    }
}

pub fn changed(previous: &[u8], next: &[u8]) -> bool {
    if previous.len() != next.len() || previous.is_empty() {
        return true;
    }
    let significant = previous
        .iter()
        .zip(next)
        .filter(|(a, b)| a.abs_diff(**b) > 24)
        .count();
    // Ignore sparse cursor blinking and minor video noise, but retain small text changes.
    significant * 1000 >= next.len() * 3
}

pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text-based stability tolerates animation behind subtitles. A second OCR
/// result is only needed if the image continues changing.
#[derive(Default)]
pub struct TextGate {
    pub text: String,
    pub revision: u64,
    since: Option<Instant>,
    confirmations: u32,
}

impl TextGate {
    pub fn observe(&mut self, text: &str, now: Instant) -> bool {
        let text = normalize(text);
        if text != self.text {
            self.text = text;
            self.revision += 1;
            self.since = Some(now);
            self.confirmations = 1;
            true
        } else {
            self.confirmations += 1;
            false
        }
    }

    pub fn ready(&self, now: Instant, image_settled: bool) -> bool {
        !self.text.is_empty()
            && self
                .since
                .is_some_and(|t| now.duration_since(t) >= STABLE_INTERVAL)
            && (image_settled || self.confirmations >= 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_overflow_and_outside_region() {
        for r in [
            Rect {
                x: u32::MAX,
                y: 0,
                width: 32,
                height: 32,
            },
            Rect {
                x: 10,
                y: 10,
                width: 100,
                height: 100,
            },
        ] {
            assert!(r.validate(100, 100).is_err());
        }
    }

    #[test]
    fn progressive_dialogue_needs_stability_and_preserves_case() {
        let now = Instant::now();
        let mut gate = TextGate::default();
        gate.observe("Hello", now);
        assert!(!gate.ready(now, true));
        gate.observe("Hello world!", now + OCR_INTERVAL);
        assert_eq!(gate.revision, 2);
        assert!(!gate.ready(now + OCR_INTERVAL, true));
        assert!(gate.ready(now + OCR_INTERVAL * 2, true));
        gate.observe("", now + OCR_INTERVAL * 3);
        assert!(!gate.ready(now + OCR_INTERVAL * 4, true));
    }

    #[test]
    fn animated_background_requires_repeated_text() {
        let mut gate = TextGate::default();
        let now = Instant::now();
        gate.observe("Use the key.", now);
        assert!(!gate.ready(now + OCR_INTERVAL, false));
        gate.observe("Use  the\nkey.", now + OCR_INTERVAL);
        assert!(gate.ready(now + OCR_INTERVAL, false));
        assert_eq!(gate.revision, 1);
    }

    #[test]
    fn noise_ignored_but_changed_text_detected() {
        let original = vec![120; 8192];
        assert!(!changed(&original, &vec![125; 8192]));
        let mut next = original.clone();
        next[100..140].fill(250);
        assert!(changed(&original, &next));
    }
}
