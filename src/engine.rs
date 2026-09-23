use crate::{
    capture::Capture,
    model::{CaptureSource, Frame, Monitor, OCR_INTERVAL, Rect, SCAN_INTERVAL, TextGate, changed},
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
    Begin(CaptureSource, Reply),
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
    frame: Option<Frame>,
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
    confirmation_frame: Option<Frame>,
    sequence: u64,
    dirty: bool,
    ocr_busy: bool,
    last_ocr: Instant,
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
                                    frame: Some(frame),
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
            confirmation_frame: None,
            sequence: 0,
            dirty: false,
            ocr_busy: false,
            last_ocr: Instant::now() - OCR_INTERVAL,
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
        self.confirmation_frame = None;
        self.fingerprint.clear();
        self.dirty = false;
        {
            let mut shared = self.shared.lock().unwrap();
            shared.last_frame_at = None;
            shared.snapshot.captured_frames = 0;
            shared.snapshot.ocr_text.clear();
            shared.snapshot.ocr_confidence = None;
            shared.snapshot.ocr_confirmations = 0;
            shared.snapshot.api_pending = false;
        }
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
            shared.snapshot.source_type = None;
            shared.snapshot.portal_position = None;
            shared.snapshot.portal_size = None;
        }
        self.show("").await;
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Begin(source, reply) => {
                if self.key.is_empty() {
                    let _ = reply.send(Err("Configure a chave do Google Cloud primeiro.".into()));
                    return;
                }
                self.stop().await;
                self.shared.lock().unwrap().snapshot.source_type = Some(source);
                self.selection_started = Instant::now();
                let (tx, rx) = oneshot::channel();
                self.cancel_open = Some(tx);
                self.opening = Some(tokio::spawn(Capture::start(source, rx)));
                self.status(
                    "opening",
                    match source {
                        CaptureSource::Monitor => {
                            "Escolha o monitor no diálogo de compartilhamento."
                        }
                        CaptureSource::Window => {
                            "Escolha a janela do emulador no diálogo de compartilhamento."
                        }
                    },
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
            capture.source.validate_layout(region, (w, h), monitor)?;
            (w, h)
        };
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
                failed => {
                    let message = match failed {
                        Ok(Err(error)) => format!("Captura não iniciada: {error}"),
                        _ => "Captura não iniciada. Tente selecionar novamente.".into(),
                    };
                    self.stop().await;
                    self.status("error", &message).await;
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
            {
                let mut shared = self.shared.lock().unwrap();
                shared.last_frame_at = Some(frame.captured);
                shared.snapshot.captured_frames += 1;
            }
            let fingerprint = frame.fingerprint();
            if changed(&self.fingerprint, &fingerprint) {
                self.fingerprint = fingerprint;
                self.sequence += 1;
                self.dirty = true;
                self.latest = Some(frame);
            } else if self.dirty {
                self.latest = Some(frame);
            } else {
                // Confirm against the freshest pixels, even when the change
                // detector found no significant difference in this frame.
                self.confirmation_frame = Some(frame);
            }
        }
        // A candidate always needs repeated OCR. Reuse the retained crop only
        // when no newer pixels are available; stop confirming once accepted.
        if !self.dirty
            && !self.ocr_busy
            && self.gate.needs_confirmation(Instant::now())
            && let Some(frame) = self.confirmation_frame.take()
        {
            self.latest = Some(frame);
            self.dirty = true;
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
        self.translate_ready(Instant::now()).await;
    }

    async fn ocr_result(&mut self, result: OcrResult) {
        self.ocr_result_at(result, Instant::now()).await;
    }

    async fn ocr_result_at(&mut self, result: OcrResult, now: Instant) {
        self.ocr_busy = false;
        if result.generation != self.generation || !self.running {
            return;
        }
        if let Some(frame) = result.frame
            && self
                .confirmation_frame
                .as_ref()
                .is_none_or(|old| old.captured < frame.captured)
        {
            self.confirmation_frame = Some(frame);
        }
        match result.result {
            Ok(output) => {
                let promoted = self.gate.observe(&output.text, now);
                {
                    let mut shared = self.shared.lock().unwrap();
                    shared.snapshot.ocr_count += 1;
                    shared.snapshot.ocr_ms = output.elapsed_ms;
                    shared.snapshot.ocr_text = output.text.clone();
                    shared.snapshot.ocr_confidence = Some(output.confidence);
                    shared.snapshot.ocr_confirmations =
                        self.gate.pending_confirmations().unwrap_or(0);
                }
                tracing::info!(
                    ocr_ms = output.elapsed_ms,
                    confidence = output.confidence,
                    generation = self.generation,
                    revision = self.gate.revision,
                    characters = output.text.chars().count(),
                    pending_confirmations = self.gate.pending_confirmations().unwrap_or(0),
                    confirmed_change = promoted,
                    "ocr"
                );
                if promoted {
                    self.text_captured = result.captured;
                    self.shown = None;
                    if let Some(task) = self.network.take() {
                        task.abort();
                        self.shared.lock().unwrap().snapshot.api_pending = false;
                    }
                    // Keep the displayed subtitle until this candidate has
                    // stabilized. One noisy/empty OCR must not make it blink.
                }
                if result.sequence == self.sequence {
                    self.dirty = false;
                }
                self.translate_ready(now).await;
            }
            Err(error) => {
                self.stop().await;
                self.status("error", &error.to_string()).await;
            }
        }
    }

    async fn translate_ready(&mut self, now: Instant) {
        if !self.running
            || self.shown == Some(self.gate.revision)
            || self.network.is_some()
            || self.dirty
            || self.ocr_busy
            || !self.gate.ready()
        {
            return;
        }
        let source = self.gate.text.clone();
        if source.is_empty() {
            self.shown = Some(self.gate.revision);
            self.show("").await;
            return;
        }
        if let Some(text) = self.cache.get(&source).cloned() {
            self.shared.lock().unwrap().snapshot.cache_hits += 1;
            self.shown = Some(self.gate.revision);
            self.show(&text).await;
            return;
        }
        // A different stable phrase has replaced the old one. Cached results
        // above replace it directly; otherwise clear while the new API runs.
        let has_subtitle = !self.shared.lock().unwrap().snapshot.translation.is_empty();
        if has_subtitle {
            self.show("").await;
        }
        if now < self.retry_at {
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
            shared.snapshot.api_pending = true;
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
        tracing::info!(generation, revision, "translation_requested");
    }

    async fn finish_network(&mut self) {
        if !self.network.as_ref().is_some_and(|task| task.is_finished()) {
            return;
        }
        self.shared.lock().unwrap().snapshot.api_pending = false;
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
                self.shared.lock().unwrap().snapshot.api_successes += 1;
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
                if !self.dirty && !self.ocr_busy && self.gate.ready() {
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

    async fn test_engine() -> Engine {
        assert_eq!(
            std::env::var("AREA_TRANSLATOR_ISOLATED_TEST").as_deref(),
            Ok("1")
        );
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::service::Shared::default()));
        let mut engine = Engine::new(shared, zbus::Connection::session().await.unwrap()).unwrap();
        engine.running = true;
        engine
    }

    async fn read(engine: &mut Engine, text: &str, now: Instant) {
        engine
            .ocr_result_at(
                OcrResult {
                    generation: engine.generation,
                    sequence: engine.sequence,
                    captured: now,
                    frame: None,
                    result: Ok(OcrOutput {
                        text: text.into(),
                        confidence: 95,
                        elapsed_ms: 1,
                    }),
                },
                now,
            )
            .await;
    }

    async fn confirm(engine: &mut Engine, text: &str, now: Instant) {
        for i in 0..3 {
            read(engine, text, now + OCR_INTERVAL * i).await;
        }
    }

    async fn finish(engine: &mut Engine) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while engine
                .network
                .as_ref()
                .is_some_and(|task| !task.is_finished())
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        engine.finish_network().await;
    }

    #[tokio::test]
    #[ignore = "run with AREA_TRANSLATOR_ISOLATED_TEST=1 under dbus-run-session"]
    async fn unstable_short_ocr_cannot_create_translation_requests_or_replace_caption() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut engine = test_engine().await;
        let count = Arc::new(AtomicUsize::new(0));
        let server_count = count.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        engine.translator =
            Translator::with_test_endpoint(format!("http://{}", listener.local_addr().unwrap()));
        engine.key = "test-only".into();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 2048];
                    let n = socket.read(&mut bytes).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&bytes[..n]);
                    let input = String::from_utf8_lossy(&request);
                    if let Some((headers, body)) = input.split_once("\r\n\r\n") {
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse()
                            .unwrap();
                        if body.len() >= length {
                            break;
                        }
                    }
                }
                server_count.fetch_add(1, Ordering::SeqCst);
                let body = r#"{"data":{"translations":[{"translatedText":"Legenda confirmada"}]}}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
        });
        let now = Instant::now();
        let variants = [
            "Are you all rlght?",
            "Are you all right? 3",
            "Are you all right!",
            "Are you all right?",
            "",
        ];
        // The former bug: merely waiting 500 ms after each different read sent it.
        for (i, text) in variants.iter().enumerate() {
            let at = now + Duration::from_secs(i as u64 * 2);
            read(&mut engine, text, at).await;
            engine.translate_ready(at + Duration::from_secs(1)).await;
        }
        assert_eq!(engine.shared.lock().unwrap().snapshot.api_count, 0);
        let stable_at = now + Duration::from_secs(20);
        confirm(&mut engine, "Are you all right?", stable_at).await;
        finish(&mut engine).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            engine.shared.lock().unwrap().snapshot.translation,
            "Legenda confirmada"
        );
        let revision = engine.gate.revision;
        for (i, text) in variants.iter().cycle().take(20).enumerate() {
            let at = stable_at + Duration::from_secs(3 + i as u64 * 3);
            read(&mut engine, text, at).await;
            read(&mut engine, text, at + OCR_INTERVAL).await;
            engine.translate_ready(at + Duration::from_secs(2)).await;
            assert_eq!(engine.gate.revision, revision);
            assert_eq!(
                engine.shared.lock().unwrap().snapshot.translation,
                "Legenda confirmada"
            );
            assert_eq!(engine.shared.lock().unwrap().snapshot.api_count, 1);
        }
        let next_at = now + Duration::from_secs(100);
        confirm(&mut engine, "Spend 11 coins.", next_at).await;
        finish(&mut engine).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        confirm(
            &mut engine,
            "Are you all right?",
            next_at + Duration::from_secs(3),
        )
        .await;
        assert_eq!(engine.shared.lock().unwrap().snapshot.api_count, 2);
        assert_eq!(engine.shared.lock().unwrap().snapshot.cache_hits, 1);
        let empty_at = next_at + Duration::from_secs(6);
        for i in 0..3 {
            read(&mut engine, "", empty_at + Duration::from_millis(750) * i).await;
        }
        assert!(
            engine
                .shared
                .lock()
                .unwrap()
                .snapshot
                .translation
                .is_empty()
        );
        confirm(
            &mut engine,
            "Are you all right?",
            next_at + Duration::from_secs(10),
        )
        .await;
        let (reply, _) = oneshot::channel();
        engine.command(Command::Pause(reply)).await;
        assert!(
            engine
                .shared
                .lock()
                .unwrap()
                .snapshot
                .translation
                .is_empty()
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        engine.stop().await;
        server.abort();
    }

    #[tokio::test]
    #[ignore = "run with AREA_TRANSLATOR_ISOLATED_TEST=1 under dbus-run-session"]
    async fn unconfirmed_noise_does_not_cancel_in_flight_translation() {
        let mut engine = test_engine().await;
        let now = Instant::now() - Duration::from_secs(2);
        for i in 0..3 {
            engine.gate.observe("Keep reading.", now + OCR_INTERVAL * i);
        }
        let revision = engine.gate.revision;
        let generation = engine.generation;
        let (reply, response) = oneshot::channel();
        engine.network = Some(tokio::spawn(async move { response.await.unwrap() }));
        for text in ["Keep readlng.", "Keep reading. 3", ""] {
            read(&mut engine, text, Instant::now()).await;
            assert!(engine.network.is_some());
            assert_eq!(engine.gate.revision, revision);
        }
        reply
            .send(NetworkResult {
                generation,
                revision,
                source: "Keep reading.".into(),
                elapsed: 350,
                result: Ok("Continue lendo.".into()),
            })
            .ok()
            .unwrap();
        finish(&mut engine).await;
        read(&mut engine, "Keep reading.", Instant::now()).await;
        assert_eq!(
            engine.shared.lock().unwrap().snapshot.translation,
            "Continue lendo."
        );
        assert_eq!(engine.shared.lock().unwrap().snapshot.api_successes, 1);
        engine.stop().await;
    }

    #[tokio::test]
    #[ignore = "requires native OCR; run with AREA_TRANSLATOR_ISOLATED_TEST=1 under dbus-run-session"]
    async fn static_image_is_confirmed_then_ocr_stops_without_new_capture_frames() {
        for name in ["dialog", "blank"] {
            let mut engine = test_engine().await;
            let expected = std::fs::read_to_string("tests/fixtures/dialog.txt").unwrap();
            let expected = expected.trim();
            engine
                .cache
                .put(expected.into(), "Legenda confirmada".into());
            if name == "blank" {
                let at = Instant::now() - Duration::from_secs(2);
                for i in 0..3 {
                    engine.gate.observe(expected, at + OCR_INTERVAL * i);
                }
                engine.show("Legenda confirmada").await;
            }
            let mut reader = png::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(format!("tests/fixtures/{name}.png")).unwrap(),
            ))
            .read_info()
            .unwrap();
            let mut gray = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut gray).unwrap();
            gray.truncate(info.buffer_size());
            engine.latest = Some(Frame {
                width: info.width,
                height: info.height,
                gray,
                captured: Instant::now(),
            });
            engine.dirty = true;
            for i in 0..3 {
                if i > 0 {
                    tokio::time::sleep(if name == "blank" {
                        Duration::from_millis(760)
                    } else {
                        Duration::from_millis(510)
                    })
                    .await;
                }
                engine.tick().await;
                assert!(engine.ocr_busy);
                let result = tokio::time::timeout(Duration::from_secs(10), engine.ocr_rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
                engine.ocr_result(result).await;
            }
            assert!(engine.gate.ready());
            assert_eq!(
                engine
                    .shared
                    .lock()
                    .unwrap()
                    .snapshot
                    .translation
                    .is_empty(),
                name == "blank"
            );
            engine.last_ocr = Instant::now() - OCR_INTERVAL;
            engine.tick().await;
            assert!(!engine.ocr_busy);
            assert_eq!(engine.shared.lock().unwrap().snapshot.ocr_count, 3);
            assert_eq!(engine.shared.lock().unwrap().snapshot.api_count, 0);
            engine.stop().await;
            assert!(engine.confirmation_frame.is_none());
        }
    }
}
