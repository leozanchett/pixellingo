use crate::{
    capture::{FrameSlot, preview_png},
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
        let (reply, receive) = oneshot::channel();
        {
            let mut state = slot.lock().unwrap();
            if state.paused || state.region.is_none() {
                return Err(zbus::fdo::Error::Failed(
                    "Inicie ou retome a tradução para conferir o recorte.".into(),
                ));
            }
            if state
                .diagnostic_request
                .as_ref()
                .is_some_and(|pending| !pending.is_closed())
            {
                return Err(zbus::fdo::Error::Failed(
                    "Já existe uma prévia em andamento.".into(),
                ));
            }
            state.diagnostic_request = Some(reply);
        }
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), receive).await
            .map_err(|_| zbus::fdo::Error::Failed("Não chegou um novo quadro. Deixe a janela capturada visível e tente novamente.".into()))?
            .map_err(|_| zbus::fdo::Error::Failed("A captura foi interrompida.".into()))?;
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
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
        assert!(frames.lock().unwrap().diagnostic_request.is_none());
        frames.lock().unwrap().paused = false;
        let deliver = async {
            let reply = loop {
                if let Some(reply) = frames.lock().unwrap().diagnostic_request.take() {
                    break reply;
                }
                tokio::task::yield_now().await;
            };
            reply
                .send(crate::model::Frame {
                    width: 16,
                    height: 16,
                    gray: vec![77; 256],
                    captured: Instant::now(),
                })
                .ok()
                .unwrap();
        };
        let (png, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(service.get_crop_preview(), deliver)
        })
        .await
        .unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(png.unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (16, 16));
        assert_eq!(info.color_type, png::ColorType::Grayscale);
        assert_eq!(pixels, vec![77; 256]);
        assert!(frames.lock().unwrap().diagnostic_request.is_none());
        assert_eq!(shared.lock().unwrap().snapshot.api_count, 0);
    }
}
