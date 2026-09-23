use crate::{
    capture::{FrameSlot, crop_now, preview_png},
    engine::Command,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use zbus::object_server::SignalEmitter;

pub const BUS: &str = "io.github.areatranslator.Service";
pub const PATH: &str = "/io/github/areatranslator/Service";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub state: String,
    pub message: String,
    pub generation: u64,
    pub revision: u64,
    pub translation: String,
    pub region: Option<crate::model::Rect>,
    pub monitor: Option<crate::model::Monitor>,
    pub frame_size: Option<(u32, u32)>,
    pub portal_position: Option<(i32, i32)>,
    pub portal_size: Option<(i32, i32)>,
    pub source_type: Option<crate::model::CaptureSource>,
    pub mode: String,
    pub manual_requests: u64,
    pub ocr_provider: String,
    pub ocr_pending: bool,
    pub ocr_successes: u64,
    pub ocr_api_ms: u64,
    pub subtitle_duration_seconds: u32,
    pub ocr_count: u64,
    pub api_count: u64,
    pub cache_hits: u64,
    pub characters_sent: u64,
    pub ocr_ms: u64,
    pub api_ms: u64,
    pub latency_ms: u64,
    pub captured_frames: u64,
    pub last_frame_age_ms: Option<u64>,
    pub ocr_text: String,
    pub ocr_confidence: Option<i32>,
    pub ocr_confirmations: u32,
    pub api_successes: u64,
    pub api_pending: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            message: "Selecione uma área para começar.".into(),
            generation: 0,
            revision: 0,
            translation: String::new(),
            region: None,
            monitor: None,
            frame_size: None,
            portal_position: None,
            portal_size: None,
            source_type: None,
            mode: "manual".into(),
            manual_requests: 0,
            ocr_provider: "google_cloud_vision".into(),
            ocr_pending: false,
            ocr_successes: 0,
            ocr_api_ms: 0,
            subtitle_duration_seconds: 15,
            ocr_count: 0,
            api_count: 0,
            cache_hits: 0,
            characters_sent: 0,
            ocr_ms: 0,
            api_ms: 0,
            latency_ms: 0,
            captured_frames: 0,
            last_frame_age_ms: None,
            ocr_text: String::new(),
            ocr_confidence: None,
            ocr_confirmations: 0,
            api_successes: 0,
            api_pending: false,
        }
    }
}

#[derive(Default)]
pub struct Shared {
    pub snapshot: Snapshot,
    pub frames: Option<FrameSlot>,
    pub last_frame_at: Option<Instant>,
}
pub type SharedState = Arc<Mutex<Shared>>;
pub type Reply = oneshot::Sender<Result<(), String>>;

pub struct Service {
    pub commands: mpsc::Sender<Command>,
    pub shared: SharedState,
}

impl Service {
    async fn request(&self, build: impl FnOnce(Reply) -> Command) -> zbus::fdo::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(build(tx))
            .await
            .map_err(|_| zbus::fdo::Error::Failed("Serviço encerrado.".into()))?;
        rx.await
            .map_err(|_| zbus::fdo::Error::Failed("Serviço encerrado.".into()))?
            .map_err(zbus::fdo::Error::Failed)
    }
}

#[zbus::interface(name = "io.github.areatranslator.Service")]
impl Service {
    async fn begin_selection(&self) -> zbus::fdo::Result<()> {
        self.request(|reply| Command::Begin(crate::model::CaptureSource::Monitor, reply))
            .await
    }
    async fn begin_window_selection(&self) -> zbus::fdo::Result<()> {
        self.request(|reply| Command::Begin(crate::model::CaptureSource::Window, reply))
            .await
    }
    async fn set_region(&self, region_json: &str, monitor_json: &str) -> zbus::fdo::Result<()> {
        let region = serde_json::from_str(region_json)
            .map_err(|_| zbus::fdo::Error::InvalidArgs("Área inválida.".into()))?;
        let monitor = serde_json::from_str(monitor_json)
            .map_err(|_| zbus::fdo::Error::InvalidArgs("Monitor inválido.".into()))?;
        self.request(|reply| Command::Region(region, monitor, reply))
            .await
    }
    async fn pause(&self) -> zbus::fdo::Result<()> {
        self.request(Command::Pause).await
    }
    async fn resume(&self) -> zbus::fdo::Result<()> {
        self.request(Command::Resume).await
    }
    async fn refresh(&self) -> zbus::fdo::Result<()> {
        self.request(Command::Refresh).await
    }
    async fn stop(&self) -> zbus::fdo::Result<()> {
        self.request(Command::Stop).await
    }
    async fn set_api_key(&self, key: &str) -> zbus::fdo::Result<()> {
        if key.len() < 10
            || key.len() > 256
            || !key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Chave de API inválida.".into(),
            ));
        }
        self.request(|reply| Command::Credential(key.into(), reply))
            .await
    }
    fn get_status(&self) -> String {
        let shared = self.shared.lock().unwrap();
        let mut snapshot = shared.snapshot.clone();
        snapshot.last_frame_age_ms = shared.last_frame_at.map(|t| t.elapsed().as_millis() as u64);
        serde_json::to_string(&snapshot).unwrap()
    }
    async fn get_preview(&self) -> zbus::fdo::Result<Vec<u8>> {
        let slot = self
            .shared
            .lock()
            .unwrap()
            .frames
            .clone()
            .ok_or_else(|| zbus::fdo::Error::Failed("Captura ainda não está pronta.".into()))?;
        tokio::task::spawn_blocking(move || preview_png(&slot))
            .await
            .map_err(|_| zbus::fdo::Error::Failed("Erro ao preparar prévia.".into()))?
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
    /// One requested crop, encoded in memory. No continuous preview or disk IO.
    async fn get_crop_preview(&self) -> zbus::fdo::Result<Vec<u8>> {
        let slot = self
            .shared
            .lock()
            .unwrap()
            .frames
            .clone()
            .ok_or_else(|| zbus::fdo::Error::Failed("Selecione uma área primeiro.".into()))?;
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let frame = crop_now(&slot)?;
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, frame.width, frame.height);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.set_compression(png::Compression::Fast);
                encoder.write_header()?.write_image_data(&frame.gray)?;
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| zbus::fdo::Error::Failed("Erro ao preparar recorte.".into()))?
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
    #[zbus(signal)]
    pub async fn status_changed(emitter: &SignalEmitter<'_>, snapshot: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    pub async fn translation_changed(
        emitter: &SignalEmitter<'_>,
        generation: u64,
        revision: u64,
        text: &str,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn crop_preview_requires_active_capture_and_encodes_only_requested_crop() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (commands, _receiver) = mpsc::channel(1);
        let service = Service {
            commands,
            shared: shared.clone(),
        };
        assert!(service.get_crop_preview().await.is_err());
        let frames = Arc::new(Mutex::new(crate::capture::SharedFrames::default()));
        {
            let mut state = frames.lock().unwrap();
            state.region = Some(crate::model::Rect {
                x: 20,
                y: 40,
                width: 16,
                height: 16,
            });
            state.paused = true;
        }
        shared.lock().unwrap().frames = Some(frames.clone());
        assert!(service.get_crop_preview().await.is_err());
        frames.lock().unwrap().paused = false;
        gstreamer::init().unwrap();
        let info = gstreamer_video::VideoInfo::builder(gstreamer_video::VideoFormat::Rgbx, 40, 60)
            .build()
            .unwrap();
        let mut buffer = gstreamer::Buffer::with_size(info.size()).unwrap();
        buffer
            .get_mut()
            .unwrap()
            .map_writable()
            .unwrap()
            .as_mut_slice()
            .fill(77);
        let sample = gstreamer::Sample::builder()
            .buffer(&buffer)
            .caps(&info.to_caps().unwrap())
            .build();
        crate::capture::retain_sample(&sample, &mut frames.lock().unwrap()).unwrap();
        let png = service.get_crop_preview().await;
        let mut reader = png::Decoder::new(std::io::Cursor::new(png.unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (16, 16));
        assert_eq!(info.color_type, png::ColorType::Grayscale);
        assert_eq!(pixels, vec![77; 256]);
        assert!(frames.lock().unwrap().frame.is_none());
        assert_eq!(shared.lock().unwrap().snapshot.api_count, 0);
    }
}
