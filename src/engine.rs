use crate::{
    capture::Capture,
    model::{Frame, Monitor, OCR_INTERVAL, Rect, SCAN_INTERVAL, TextGate, changed},
    ocr::{Ocr, OcrOutput},
    service::{PATH, Reply, Service, SharedState},
    translate::{TranslationError, Translator},
};
use lru::LruCache;
use std::{
    num::NonZeroUsize,
    sync::mpsc as sync,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

pub enum Command {
    Begin(Reply),
    Region(Rect, Monitor, Reply),
    Pause(Reply),
    Resume(Reply),
    Stop(Reply),
    Credential(String, Reply),
}
enum OcrJob {
    Frame(u64, u64, Frame),
    Reset,
}
struct OcrResult {
    generation: u64,
    sequence: u64,
    captured: Instant,
    result: anyhow::Result<OcrOutput>,
}
struct NetworkResult {
    generation: u64,
    revision: u64,
    source: String,
    elapsed: u64,
    result: Result<String, TranslationError>,
}

pub struct Engine {
    shared: SharedState,
    connection: zbus::Connection,
    capture: Option<Capture>,
    opening: Option<JoinHandle<anyhow::Result<Capture>>>,
    cancel_open: Option<oneshot::Sender<()>>,
    key: String,
    generation: u64,
    running: bool,
    gate: TextGate,
    fingerprint: Vec<u8>,
    latest: Option<Frame>,
    sequence: u64,
    dirty: bool,
    ocr_busy: bool,
    last_ocr: Instant,
    last_image_change: Instant,
    text_captured: Instant,
    ocr_tx: sync::SyncSender<OcrJob>,
    ocr_rx: mpsc::Receiver<OcrResult>,
    network: Option<JoinHandle<NetworkResult>>,
    translator: Translator,
    cache: LruCache<String, String>,
    shown: Option<u64>,
    failures: u32,
    retry_at: Instant,
    selection_started: Instant,
}

impl Engine {
    pub fn new(shared: SharedState, connection: zbus::Connection) -> anyhow::Result<Self> {
        let (ocr_tx, jobs) = sync::sync_channel(1);
        let (results, ocr_rx) = mpsc::channel(1);
        std::thread::Builder::new()
            .name("area-ocr".into())
            .spawn(move || {
                let mut ocr = None;
                while let Ok(job) = jobs.recv() {
                    match job {
                        OcrJob::Reset => ocr = None,
                        OcrJob::Frame(generation, sequence, frame) => {
                            let result = (|| {
                                if ocr.is_none() {
                                    ocr = Some(Ocr::new()?);
                                }
                                ocr.as_mut().unwrap().recognize(&frame)
                            })();
                            if results
                                .blocking_send(OcrResult {
                                    generation,
                                    sequence,
                                    captured: frame.captured,
                                    result,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            shared,
            connection,
            capture: None,
            opening: None,
            cancel_open: None,
            key: String::new(),
            generation: 0,
            running: false,
            gate: TextGate::default(),
            fingerprint: vec![],
            latest: None,
            sequence: 0,
            dirty: false,
            ocr_busy: false,
            last_ocr: Instant::now() - OCR_INTERVAL,
            last_image_change: Instant::now(),
            text_captured: Instant::now(),
            ocr_tx,
            ocr_rx,
            network: None,
            translator: Translator::new()?,
            cache: LruCache::new(NonZeroUsize::new(2000).unwrap()),
            shown: None,
            failures: 0,
            retry_at: Instant::now(),
            selection_started: Instant::now(),
        })
    }

    pub async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        loop {
            let interval = if self.running || self.opening.is_some() || self.capture.is_some() {
                SCAN_INTERVAL
            } else {
                Duration::from_secs(60)
            };
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.command(command).await,
                    None => break,
                },
                Some(result) = self.ocr_rx.recv() => self.ocr_result(result).await,
                _ = tokio::time::sleep(interval) => self.tick().await,
                _ = tokio::signal::ctrl_c() => break,
            }
        }
        self.stop().await;
    }

    async fn status(&self, state: &str, message: &str) {
        let snapshot = {
            let mut shared = self.shared.lock().unwrap();
            shared.snapshot.state = state.into();
            shared.snapshot.message = message.into();
            shared.snapshot.generation = self.generation;
            shared.snapshot.revision = self.gate.revision;
            serde_json::to_string(&shared.snapshot).unwrap()
        };
        if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&self.connection, PATH) {
            let _ = Service::status_changed(&emitter, &snapshot).await;
        }
    }

    async fn show(&self, text: &str) {
        {
            let mut shared = self.shared.lock().unwrap();
            shared.snapshot.translation = text.into();
            shared.snapshot.revision = self.gate.revision;
        }
        if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&self.connection, PATH) {
            let _ =
                Service::translation_changed(&emitter, self.generation, self.gate.revision, text)
                    .await;
        }
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.gate = TextGate::default();
        self.shown = None;
        self.latest = None;
        self.fingerprint.clear();
        self.dirty = false;
        if let Some(task) = self.network.take() {
            task.abort();
        }
    }

    async fn stop(&mut self) {
        self.running = false;
        self.invalidate();
        if let Some(cancel) = self.cancel_open.take() {
            let _ = cancel.send(());
        }
        if let Some(task) = self.opening.take()
            && let Ok(Ok(capture)) = task.await
        {
            capture.close().await;
        }
        if let Some(capture) = self.capture.take() {
            capture.close().await;
        }
        let _ = self.ocr_tx.try_send(OcrJob::Reset);
        {
            let mut shared = self.shared.lock().unwrap();
            shared.frames = None;
            shared.snapshot.region = None;
            shared.snapshot.monitor = None;
            shared.snapshot.frame_size = None;
        }
        self.show("").await;
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Begin(reply) => {
                if self.key.is_empty() {
                    let _ = reply.send(Err("Configure a chave do Google Cloud primeiro.".into()));
                    return;
                }
                self.stop().await;
                self.selection_started = Instant::now();
                let (tx, rx) = oneshot::channel();
                self.cancel_open = Some(tx);
                self.opening = Some(tokio::spawn(Capture::start(rx)));
                self.status(
                    "opening",
                    "Escolha o monitor no diálogo de compartilhamento.",
                )
                .await;
                let _ = reply.send(Ok(()));
            }
            Command::Region(region, monitor, reply) => {
                let result = self.select_region(region, monitor).await;
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Command::Pause(reply) => {
                self.running = false;
                self.invalidate();
                let result = self
                    .capture
                    .as_ref()
                    .map(|c| c.pause(true))
                    .unwrap_or(Ok(()));
                self.show("").await;
                self.status("paused", "Tradução pausada.").await;
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Command::Resume(reply) => {
                let has_region = self.shared.lock().unwrap().snapshot.region.is_some();
                let result = if has_region {
                    self.capture
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Selecione uma área."))
                        .and_then(|c| c.pause(false))
                } else {
                    Err(anyhow::anyhow!("Selecione uma área."))
                };
                if result.is_ok() {
                    self.invalidate();
                    self.running = true;
                    self.failures = 0;
                    self.retry_at = Instant::now();
                    self.status("running", "Observando a área selecionada.")
                        .await;
                }
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Command::Stop(reply) => {
                self.stop().await;
                self.status("idle", "Captura encerrada.").await;
                let _ = reply.send(Ok(()));
            }
            Command::Credential(key, reply) => {
                self.key = key;
                self.failures = 0;
                self.retry_at = Instant::now();
                let _ = reply.send(Ok(()));
            }
        }
    }

    async fn select_region(&mut self, region: Rect, monitor: Monitor) -> anyhow::Result<()> {
        anyhow::ensure!(
            (100..=16384).contains(&monitor.width) && (100..=16384).contains(&monitor.height),
            "Monitor inválido."
        );
        let capture = self
            .capture
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Abra a captura primeiro."))?;
        let dimensions = {
            let state = capture.slot.lock().unwrap();
            let (w, h) = state
                .dimensions
                .ok_or_else(|| anyhow::anyhow!("Aguardando primeiro quadro."))?;
            region.validate(w, h)?;
            (w, h)
        };
        // A subtitle needs space outside the OCR rectangle, above or below.
        let top = region.y as f64 / dimensions.1 as f64 * monitor.height as f64;
        let bottom =
            (region.y + region.height) as f64 / dimensions.1 as f64 * monitor.height as f64;
        anyhow::ensure!(
            top >= 110.0 || monitor.height as f64 - bottom >= 110.0,
            "Deixe pelo menos 110 pixels livres acima ou abaixo da área para a legenda."
        );
        self.invalidate();
        let capture = self.capture.as_ref().unwrap();
        {
            let mut state = capture.slot.lock().unwrap();
            state.region = Some(region);
            state.preview = None;
            state.frame = None;
        }
        capture.pause(false)?;
        {
            let mut shared = self.shared.lock().unwrap();
            shared.snapshot.region = Some(region);
            shared.snapshot.monitor = Some(monitor);
            shared.snapshot.frame_size = Some(dimensions);
        }
        self.running = true;
        self.failures = 0;
        self.retry_at = Instant::now();
        self.status("running", "Observando a área selecionada.")
            .await;
        Ok(())
    }

    async fn tick(&mut self) {
        if self.opening.as_ref().is_some_and(|task| task.is_finished()) {
            let result = self.opening.take().unwrap().await;
            self.cancel_open = None;
            match result {
                Ok(Ok(capture)) => {
                    {
                        let mut shared = self.shared.lock().unwrap();
                        shared.frames = Some(capture.slot.clone());
                        shared.snapshot.portal_position = capture.position;
                        shared.snapshot.portal_size = capture.logical_size;
                    }
                    self.capture = Some(capture);
                    self.selection_started = Instant::now();
                    self.status("selecting", "Marque a região na prévia.").await;
                }
                _ => {
                    self.stop().await;
                    self.status(
                        "error",
                        "Captura não iniciada. Tente selecionar o monitor novamente.",
                    )
                    .await;
                }
            }
        }
        if let Some(error) = self.capture.as_ref().and_then(|c| c.error()) {
            self.stop().await;
            self.status("error", &error).await;
            return;
        }
        // Forgotten selection windows must not retain a screen share forever.
        if self.capture.is_some()
            && !self.running
            && self.shared.lock().unwrap().snapshot.state == "selecting"
            && self.selection_started.elapsed() > Duration::from_secs(300)
        {
            self.stop().await;
            self.status("idle", "Seleção expirou. Selecione a área novamente.")
                .await;
        }
        if !self.running {
            return;
        }
        let frame = self
            .capture
            .as_ref()
            .and_then(|c| c.slot.lock().unwrap().frame.take());
        if let Some(frame) = frame {
            let fingerprint = frame.fingerprint();
            if changed(&self.fingerprint, &fingerprint) {
                self.fingerprint = fingerprint;
                self.sequence += 1;
                self.last_image_change = Instant::now();
                self.dirty = true;
                self.latest = Some(frame);
            }
        }
        if self.dirty
            && !self.ocr_busy
            && self.last_ocr.elapsed() >= OCR_INTERVAL
            && let Some(frame) = self.latest.take()
        {
            match self
                .ocr_tx
                .try_send(OcrJob::Frame(self.generation, self.sequence, frame))
            {
                Ok(()) => {
                    self.ocr_busy = true;
                    self.last_ocr = Instant::now();
                    self.dirty = false;
                }
                Err(sync::TrySendError::Full(OcrJob::Frame(_, _, frame))) => {
                    self.latest = Some(frame)
                }
                _ => {
                    self.stop().await;
                    self.status("error", "Trabalhador de OCR indisponível.")
                        .await;
                    return;
                }
            }
        }
        self.finish_network().await;
        self.translate_ready().await;
    }

    async fn ocr_result(&mut self, result: OcrResult) {
        self.ocr_busy = false;
        if result.generation != self.generation || !self.running {
            return;
        }
        match result.result {
            Ok(output) => {
                {
                    let mut shared = self.shared.lock().unwrap();
                    shared.snapshot.ocr_count += 1;
                    shared.snapshot.ocr_ms = output.elapsed_ms;
                }
                tracing::info!(
                    ocr_ms = output.elapsed_ms,
                    confidence = output.confidence,
                    "ocr"
                );
                if self.gate.observe(&output.text, Instant::now()) {
                    self.text_captured = result.captured;
                    self.shown = None;
                    if let Some(task) = self.network.take() {
                        task.abort();
                    }
                    self.show("").await;
                }
                if result.sequence == self.sequence {
                    self.dirty = false;
                }
                self.translate_ready().await;
            }
            Err(error) => {
                self.stop().await;
                self.status("error", &error.to_string()).await;
            }
        }
    }

    async fn translate_ready(&mut self) {
        if !self.running
            || self.shown == Some(self.gate.revision)
            || self.network.is_some()
            || self.dirty
            || self.ocr_busy
            || !self.gate.ready(
                Instant::now(),
                self.last_image_change.elapsed() >= crate::model::STABLE_INTERVAL,
            )
        {
            return;
        }
        let source = self.gate.text.clone();
        if let Some(text) = self.cache.get(&source).cloned() {
            self.shared.lock().unwrap().snapshot.cache_hits += 1;
            self.shown = Some(self.gate.revision);
            self.show(&text).await;
            return;
        }
        if Instant::now() < self.retry_at {
            return;
        }
        // Bound request size; noise/huge selections must not create expensive calls.
        if source.chars().count() > 4000 {
            self.running = false;
            if let Some(capture) = &self.capture {
                let _ = capture.pause(true);
            }
            self.status("blocked", "Texto muito longo. Selecione uma área menor.")
                .await;
            return;
        }
        let translator = self.translator.clone();
        let key = self.key.clone();
        let generation = self.generation;
        let revision = self.gate.revision;
        {
            let mut shared = self.shared.lock().unwrap();
            shared.snapshot.api_count += 1;
            shared.snapshot.characters_sent += source.chars().count() as u64;
        }
        self.network = Some(tokio::spawn(async move {
            let start = Instant::now();
            let result = translator.translate(&key, &source).await;
            NetworkResult {
                generation,
                revision,
                source,
                elapsed: start.elapsed().as_millis() as u64,
                result,
            }
        }));
    }

    async fn finish_network(&mut self) {
        if !self.network.as_ref().is_some_and(|task| task.is_finished()) {
            return;
        }
        let Ok(result) = self.network.take().unwrap().await else {
            return;
        };
        if !is_current(
            self.generation,
            self.gate.revision,
            result.generation,
            result.revision,
        ) {
            return;
        }
        self.shared.lock().unwrap().snapshot.api_ms = result.elapsed;
        match result.result {
            Ok(text) => {
                self.cache.put(result.source, text.clone());
                self.failures = 0;
                self.retry_at = Instant::now();
                self.shared.lock().unwrap().snapshot.latency_ms =
                    self.text_captured.elapsed().as_millis() as u64;
                tracing::info!(
                    api_ms = result.elapsed,
                    latency_ms = self.text_captured.elapsed().as_millis() as u64,
                    "translation"
                );
                if !self.dirty && !self.ocr_busy {
                    self.shown = Some(self.gate.revision);
                    self.show(&text).await;
                }
                self.status("running", "Observando a área selecionada.")
                    .await;
            }
            Err(error) => {
                if error.suspends() {
                    self.running = false;
                    if let Some(capture) = &self.capture {
                        let _ = capture.pause(true);
                    }
                    self.show("").await;
                    self.status("blocked", error.message()).await;
                } else {
                    self.failures = (self.failures + 1).min(5);
                    self.retry_at = Instant::now() + Duration::from_secs(1 << self.failures);
                    self.status("retrying", error.message()).await;
                }
            }
        }
    }
}

pub fn is_current(
    generation: u64,
    revision: u64,
    result_generation: u64,
    result_revision: u64,
) -> bool {
    generation == result_generation && revision == result_revision
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_results_after_pause_region_change_and_dialogue_change_are_rejected() {
        assert!(is_current(2, 5, 2, 5));
        assert!(!is_current(2, 5, 1, 5));
        assert!(!is_current(2, 5, 2, 4));
    }
}
