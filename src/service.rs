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
        self.request(Command::Begin).await
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
