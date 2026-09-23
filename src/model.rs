use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const SCAN_INTERVAL: Duration = Duration::from_millis(200);
pub const OCR_INTERVAL: Duration = Duration::from_millis(500);
pub const STABLE_INTERVAL: Duration = Duration::from_millis(500);
pub const EMPTY_INTERVAL: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaptureSource {
    Monitor,
    Window,
}

impl CaptureSource {
    pub fn validate_layout(
        self,
        region: Rect,
        dimensions: (u32, u32),
        monitor: Monitor,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            (100..=16384).contains(&monitor.width) && (100..=16384).contains(&monitor.height),
            "Monitor inválido."
        );
        region.validate(dimensions.0, dimensions.1)?;
        if self == Self::Monitor {
            let top = region.y as f64 / dimensions.1 as f64 * monitor.height as f64;
            let bottom =
                (region.y + region.height) as f64 / dimensions.1 as f64 * monitor.height as f64;
            anyhow::ensure!(
                top >= 110.0 || monitor.height as f64 - bottom >= 110.0,
                "Deixe pelo menos 110 pixels livres acima ou abaixo da área para a legenda."
            );
        }
        // A window stream excludes Shell chrome. Its crop coordinates cannot be
        // projected onto the output monitor, and require no subtitle exclusion.
        Ok(())
    }
}

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

/// Small OCR artifacts must not restart stability forever on animated scenes.
/// Keep the comparison bounded, anchor it to the representative (no drift), and
/// do not merge short messages or changes to numbers/negations.
fn similar_ocr(a: &str, b: &str) -> bool {
    fn words(text: &str) -> Vec<String> {
        text.split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
                    .replace('’', "'")
            })
            .filter(|w| !w.is_empty())
            .collect()
    }
    fn significant(words: &[String]) -> Vec<&str> {
        words
            .iter()
            .filter(|w| {
                w.chars().any(|c| c.is_numeric())
                    || matches!(w.as_str(), "no" | "not" | "never" | "without" | "cannot")
                    || w.ends_with("n't")
            })
            .map(String::as_str)
            .collect()
    }
    let aw = words(a);
    let bw = words(b);
    if aw.len().min(bw.len()) < 8 || significant(&aw) != significant(&bw) {
        return false;
    }
    let a: Vec<char> = aw.join(" ").chars().collect();
    let b: Vec<char> = bw.join(" ").chars().collect();
    let length = a.len().min(b.len());
    if length < 60 || a.len().max(b.len()) > 4000 {
        return false;
    }
    let limit = (length / 16).min(8);
    if a.len().abs_diff(b.len()) > limit {
        return false;
    }
    // Banded edit distance: at most 17 cells per source character.
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut row = vec![limit + 1; b.len() + 1];
    for (index, &ch) in a.iter().enumerate() {
        let i = index + 1;
        let start = i.saturating_sub(limit).max(1);
        let end = (i + limit).min(b.len());
        row[0] = i;
        if start > 1 {
            row[start - 1] = limit + 1;
        }
        let mut minimum = limit + 1;
        for j in start..=end {
            row[j] = (previous[j] + 1)
                .min(row[j - 1] + 1)
                .min(previous[j - 1] + usize::from(ch != b[j - 1]));
            minimum = minimum.min(row[j]);
        }
        if end < b.len() {
            row[end + 1] = limit + 1;
        }
        if minimum > limit {
            return false;
        }
        std::mem::swap(&mut previous, &mut row);
    }
    previous[b.len()] <= limit
}

/// Text-based stability tolerates animation behind subtitles. A second OCR
/// result is needed if the image keeps changing or to confirm empty text.
#[derive(Default)]
pub struct TextGate {
    pub text: String,
    pub revision: u64,
    since: Option<Instant>,
    confirmations: u32,
    variant: Option<(String, Instant)>,
}

impl TextGate {
    pub fn observe(&mut self, text: &str, now: Instant) -> bool {
        let text = normalize(text);
        if text != self.text && similar_ocr(&self.text, &text) {
            self.confirmations = self.confirmations.saturating_add(1);
            if let Some((variant, since)) = &self.variant
                && variant == &text
                && now.duration_since(*since) >= STABLE_INTERVAL
            {
                // A persistent near-match may be a real one-word change, not
                // noise. Adopt it after repeated identical observations.
                self.since = Some(*since);
                self.text = text;
                self.revision += 1;
                self.variant = None;
                return true;
            }
            if self
                .variant
                .as_ref()
                .is_none_or(|(variant, _)| variant != &text)
            {
                self.variant = Some((text, now));
            }
            return false;
        }
        self.variant = None;
        if text != self.text {
            self.text = text;
            self.revision += 1;
            self.since = Some(now);
            self.confirmations = 1;
            true
        } else {
            self.confirmations = self.confirmations.saturating_add(1);
            false
        }
    }

    pub fn ready(&self, now: Instant, image_settled: bool) -> bool {
        // Empty/low-confidence reads need a longer grace period so a single
        // missed frame does not erase a subtitle the user is still reading.
        let interval = if self.text.is_empty() {
            EMPTY_INTERVAL
        } else {
            STABLE_INTERVAL
        };
        self.since
            .is_some_and(|t| now.duration_since(t) >= interval)
            && if self.text.is_empty() {
                self.confirmations >= 2
            } else {
                image_settled || self.confirmations >= 2
            }
    }

    pub fn needs_confirmation(&self, now: Instant) -> bool {
        self.variant
            .as_ref()
            .is_some_and(|(_, since)| now.duration_since(*since) >= STABLE_INTERVAL)
            || (self.text.is_empty()
                && self.confirmations < 2
                && self
                    .since
                    .is_some_and(|t| now.duration_since(t) >= EMPTY_INTERVAL))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_crop_is_independent_of_overlay_monitor_geometry() {
        let region = Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        };
        let monitor = Monitor {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert!(
            CaptureSource::Window
                .validate_layout(region, (640, 480), monitor)
                .is_ok()
        );
        assert!(
            CaptureSource::Monitor
                .validate_layout(region, (640, 480), monitor)
                .is_err()
        );
        assert!(
            CaptureSource::Window
                .validate_layout(region, (320, 240), monitor)
                .is_err()
        );
        assert!(
            CaptureSource::Window
                .validate_layout(
                    region,
                    (640, 480),
                    Monitor {
                        width: 0,
                        ..monitor
                    }
                )
                .is_err()
        );
    }

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
    fn empty_readings_require_a_longer_stable_absence() {
        let now = Instant::now();
        let mut gate = TextGate::default();
        assert!(!gate.ready(now + EMPTY_INTERVAL, true));
        gate.observe("Keep reading.", now);
        gate.observe("", now + STABLE_INTERVAL);
        let cleared_at = now + STABLE_INTERVAL;
        assert!(!gate.ready(cleared_at + STABLE_INTERVAL, true));
        assert!(!gate.ready(cleared_at + EMPTY_INTERVAL, false));
        assert!(!gate.ready(cleared_at + EMPTY_INTERVAL, true));
        assert!(gate.needs_confirmation(cleared_at + EMPTY_INTERVAL));
        gate.observe("", cleared_at + EMPTY_INTERVAL);
        assert!(gate.ready(cleared_at + EMPTY_INTERVAL, false));
        assert!(!gate.needs_confirmation(cleared_at + EMPTY_INTERVAL));
    }

    #[test]
    fn small_changing_ocr_artifacts_do_not_starve_translation() {
        let phrase = "There is no saved game on the memory card. Would you like to create a new file? Yes No";
        let mut gate = TextGate::default();
        let now = Instant::now();
        assert!(gate.observe(phrase, now));
        let revision = gate.revision;
        for (index, noise) in ["l", "r", "I", "v", "l"].iter().enumerate() {
            let at = now + OCR_INTERVAL * (index as u32 + 1);
            assert!(!gate.observe(&format!("{noise} {phrase} |"), at));
            assert!(
                gate.ready(at, false),
                "Animated background with small artifacts must become ready"
            );
            assert_eq!(gate.revision, revision);
            assert_eq!(
                gate.text, phrase,
                "Anchor must not drift through successive near-matches"
            );
        }
    }

    #[test]
    fn numbers_negations_and_short_messages_are_not_merged_as_ocr_noise() {
        let phrase =
            "You have 10 arrows in the inventory. Do not leave the village without your shield.";
        assert!(!similar_ocr(phrase, &phrase.replace("10", "11")));
        assert!(!similar_ocr(phrase, &phrase.replace("not ", "")));
        assert!(!similar_ocr("Go east.", "Go west."));
        assert!(!similar_ocr(
            phrase,
            "A completely different conversation now appears inside this dialogue box."
        ));
        assert!(!similar_ocr(phrase, ""));
    }

    #[test]
    fn repeated_near_match_is_adopted_as_a_real_change() {
        let phrase =
            "Follow the path to the east until you reach the stone bridge near the village.";
        let changed = phrase.replace("east", "west");
        assert!(similar_ocr(phrase, &changed));
        let now = Instant::now();
        let mut gate = TextGate::default();
        gate.observe(phrase, now);
        assert!(!gate.observe(&changed, now + OCR_INTERVAL));
        assert!(gate.needs_confirmation(now + OCR_INTERVAL * 2));
        assert!(gate.observe(&changed, now + OCR_INTERVAL * 2));
        assert!(!gate.needs_confirmation(now + OCR_INTERVAL * 2));
        assert_eq!(gate.text, changed);
        assert_eq!(gate.revision, 2);
        assert!(gate.ready(now + OCR_INTERVAL * 2, false));
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
