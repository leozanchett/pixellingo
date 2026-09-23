use serde::{Deserialize, Serialize};
use std::time::Instant;

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

pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
}
