use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const SCAN_INTERVAL: Duration = Duration::from_millis(200);
pub const OCR_INTERVAL: Duration = Duration::from_millis(500);
pub const STABLE_INTERVAL: Duration = Duration::from_millis(1000);
pub const TEXT_CONFIRMATIONS: u32 = 3;
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

/// Only confirmed text owns a revision. Candidates cannot cancel an in-flight
/// translation or replace its cache key until three consecutive readings agree.
#[derive(Default)]
pub struct TextGate {
    pub text: String,
    pub revision: u64,
    pending: Option<TextCandidate>,
}

struct TextCandidate {
    text: String,
    since: Instant,
    last_read: Instant,
    confirmations: u32,
}

impl TextGate {
    pub fn observe(&mut self, text: &str, now: Instant) -> bool {
        let text = normalize(text);
        if text == self.text && self.revision > 0 {
            self.pending = None;
            return false;
        }
        match &mut self.pending {
            Some(candidate) if candidate.text == text => {
                candidate.confirmations = candidate.confirmations.saturating_add(1);
                candidate.last_read = now;
            }
            _ => {
                self.pending = Some(TextCandidate {
                    text,
                    since: now,
                    last_read: now,
                    confirmations: 1,
                })
            }
        }
        let candidate = self.pending.as_ref().unwrap();
        let interval = if candidate.text.is_empty() {
            EMPTY_INTERVAL
        } else {
            STABLE_INTERVAL
        };
        if candidate.confirmations < TEXT_CONFIRMATIONS
            || now.duration_since(candidate.since) < interval
        {
            return false;
        }
        self.text = self.pending.take().unwrap().text;
        self.revision += 1;
        true
    }

    pub fn ready(&self) -> bool {
        self.revision > 0 && self.pending.is_none()
    }

    pub fn pending_confirmations(&self) -> Option<u32> {
        self.pending
            .as_ref()
            .map(|candidate| candidate.confirmations)
    }

    pub fn needs_confirmation(&self, now: Instant) -> bool {
        self.pending.as_ref().is_some_and(|candidate| {
            // Empty text uses a longer spacing to cover its 1.5 second grace.
            let spacing = if candidate.text.is_empty() {
                EMPTY_INTERVAL / 2
            } else {
                OCR_INTERVAL
            };
            now.duration_since(candidate.last_read) >= spacing
        })
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

    fn confirm(gate: &mut TextGate, text: &str, at: Instant) {
        assert!(!gate.observe(text, at));
        assert!(!gate.observe(text, at + OCR_INTERVAL));
        assert!(gate.observe(text, at + STABLE_INTERVAL));
    }

    #[test]
    fn one_read_never_becomes_ready_just_because_time_passes() {
        let now = Instant::now();
        let mut gate = TextGate::default();
        gate.observe("Are you all right?", now);
        assert!(!gate.ready());
        assert!(gate.needs_confirmation(now + Duration::from_secs(3600)));
        assert_eq!(gate.revision, 0);
        gate.observe("Are you all right?", now + OCR_INTERVAL);
        assert!(!gate.ready());
        assert!(gate.observe("Are you all right?", now + STABLE_INTERVAL));
        assert!(gate.ready());
    }

    #[test]
    fn fluctuating_short_and_long_text_never_replaces_confirmed_text() {
        let now = Instant::now();
        for original in [
            "Are you all right?",
            "There is no saved game on the memory card. Would you like to create a new file?",
        ] {
            let mut gate = TextGate::default();
            confirm(&mut gate, original, now);
            for (i, text) in [
                format!("{original} 3"),
                original.replace('i', "l"),
                original.into(),
                String::new(),
                format!("{original} |"),
                original.into(),
            ]
            .iter()
            .enumerate()
            {
                let at = now + Duration::from_secs(3 + i as u64 * 2);
                assert!(!gate.observe(text, at));
                assert_eq!(gate.text, original);
                assert_eq!(gate.revision, 1);
            }
            assert!(gate.ready());
        }
    }

    #[test]
    fn two_alternating_variants_do_not_form_a_majority_loop() {
        let now = Instant::now();
        let mut gate = TextGate::default();
        for i in 0..20 {
            assert!(!gate.observe(
                if i % 2 == 0 { "Go east." } else { "Go west." },
                now + OCR_INTERVAL * i
            ));
            assert!(!gate.ready());
        }
        assert_eq!(gate.revision, 0);
    }

    #[test]
    fn numbers_negations_and_small_real_changes_are_confirmed_not_merged() {
        let now = Instant::now();
        let mut gate = TextGate::default();
        for (i, phrase) in [
            "Go east.",
            "Go west.",
            "Spend 10 coins.",
            "Spend 11 coins.",
            "Do not enter.",
            "Do enter.",
        ]
        .iter()
        .enumerate()
        {
            confirm(&mut gate, phrase, now + Duration::from_secs(i as u64 * 3));
            assert_eq!(gate.text, *phrase);
            assert_eq!(gate.revision, i as u64 + 1);
        }
    }

    #[test]
    fn empty_readings_require_repetition_and_longer_stable_absence() {
        let now = Instant::now();
        let mut gate = TextGate::default();
        confirm(&mut gate, "Keep reading.", now);
        let at = now + Duration::from_secs(2);
        assert!(!gate.observe("", at));
        assert!(!gate.observe("", at + OCR_INTERVAL));
        assert!(!gate.observe("", at + STABLE_INTERVAL));
        assert_eq!(gate.text, "Keep reading.");
        assert!(gate.observe("", at + EMPTY_INTERVAL));
        assert_eq!(gate.text, "");
        assert!(gate.ready());
        assert!(!gate.needs_confirmation(at + Duration::from_secs(10)));
    }

    #[test]
    fn progressive_dialogue_and_whitespace_need_repeated_observations() {
        let now = Instant::now();
        let mut gate = TextGate::default();
        gate.observe("Hello", now);
        gate.observe("Hello world!", now + OCR_INTERVAL);
        gate.observe("Hello  world!\n", now + OCR_INTERVAL * 2);
        assert!(!gate.ready());
        assert!(gate.observe("Hello world!", now + OCR_INTERVAL * 3));
        assert_eq!(gate.text, "Hello world!");
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
