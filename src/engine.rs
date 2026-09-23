use crate::{
    capture::{Capture, crop_now},
    model::{CaptureSource, Frame, Monitor, Rect, normalize},
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
    Refresh(Reply),
    Stop(Reply),
    Credential(String, Reply),
}
enum OcrJob {
    Frame(u64, Frame),
    Reset,
}
struct OcrResult {
    generation: u64,
    captured: Instant,
    result: anyhow::Result<OcrOutput>,
}
struct NetworkResult {
    generation: u64,
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
    pending_frame: Option<Frame>,
    manual_pending: bool,
    ocr_busy: bool,
    text_captured: Instant,
    ocr_tx: sync::SyncSender<OcrJob>,
    ocr_rx: mpsc::Receiver<OcrResult>,
    network: Option<JoinHandle<NetworkResult>>,
    translator: Translator,
    cache: LruCache<String, String>,
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
                        OcrJob::Frame(generation, frame) => {
                            let result = (|| {
                                if ocr.is_none() {
                                    ocr = Some(Ocr::new()?);
                                }
                                ocr.as_mut().unwrap().recognize(&frame)
                            })();
                            if results
                                .blocking_send(OcrResult {
                                    generation,
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
            pending_frame: None,
            manual_pending: false,
            ocr_busy: false,
            text_captured: Instant::now(),
            ocr_tx,
            ocr_rx,
            network: None,
            translator: Translator::new()?,
            cache: LruCache::new(NonZeroUsize::new(2000).unwrap()),
            failures: 0,
            retry_at: Instant::now(),
            selection_started: Instant::now(),
        })
    }

    pub async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        loop {
            let interval = if self.manual_pending || self.opening.is_some() {
                Duration::from_millis(100)
            } else if self.capture.is_some() {
                Duration::from_secs(1)
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
            shared.snapshot.revision = self.generation;
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
            shared.snapshot.revision = self.generation;
        }
        if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&self.connection, PATH) {
            let _ = Service::translation_changed(&emitter, self.generation, self.generation, text)
                .await;
        }
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.pending_frame = None;
        self.manual_pending = false;
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
                    self.status("running", "Modo manual: use o atalho ou Traduzir agora.")
                        .await;
                }
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Command::Stop(reply) => {
                self.stop().await;
                self.status("idle", "Captura encerrada.").await;
                let _ = reply.send(Ok(()));
            }
            Command::Refresh(reply) => {
                let result = self.refresh().await;
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            Command::Credential(key, reply) => {
                self.key = key;
                self.failures = 0;
                self.retry_at = Instant::now();
                let _ = reply.send(Ok(()));
            }
        }
    }

    async fn refresh(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.shared.lock().unwrap().snapshot.region.is_some(),
            "Abra o tradutor e selecione a área antes de usar o atalho."
        );
        anyhow::ensure!(self.running, "Retome a captura antes de traduzir.");
        // Do not queue repeated key events or duplicate a request already running.
        if self.manual_pending {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() >= self.retry_at,
            "Aguarde {} segundos e pressione o atalho novamente.",
            self.retry_at
                .saturating_duration_since(Instant::now())
                .as_secs()
                + 1
        );
        let slot = self
            .shared
            .lock()
            .unwrap()
            .frames
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Selecione a área novamente."))?;
        let frame = crop_now(&slot)?;
        self.invalidate();
        self.text_captured = frame.captured;
        {
            let mut shared = self.shared.lock().unwrap();
            shared.snapshot.manual_requests += 1;
            shared.snapshot.captured_frames += 1;
            shared.last_frame_at = Some(frame.captured);
        }
        self.manual_pending = true;
        self.pending_frame = Some(frame);
        // Preserve the last successful subtitle until a replacement is ready.
        self.status("running", "Lendo a captura solicitada…").await;
        tracing::info!(generation = self.generation, "manual_translation_requested");
        self.process_pending().await;
        Ok(())
    }

    async fn process_pending(&mut self) {
        if !self.running || !self.manual_pending || self.ocr_busy {
            return;
        }
        let Some(frame) = self.pending_frame.take() else {
            return;
        };
        match self.ocr_tx.try_send(OcrJob::Frame(self.generation, frame)) {
            Ok(()) => self.ocr_busy = true,
            Err(sync::TrySendError::Full(OcrJob::Frame(_, frame))) => {
                self.pending_frame = Some(frame)
            }
            _ => {
                self.manual_pending = false;
                self.status(
                    "running",
                    "OCR indisponível. Encerre e abra o tradutor novamente.",
                )
                .await;
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
        self.show("").await;
        self.running = true;
        self.failures = 0;
        self.retry_at = Instant::now();
        self.status("running", "Modo manual: use o atalho ou Traduzir agora.")
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
        self.process_pending().await;
        self.finish_network().await;
    }

    async fn ocr_result(&mut self, result: OcrResult) {
        self.ocr_busy = false;
        if result.generation != self.generation || !self.running || !self.manual_pending {
            return;
        }
        let output = match result.result {
            Ok(output) => output,
            Err(_) => {
                self.manual_pending = false;
                self.status(
                    "running",
                    "Falha no OCR. Pressione o atalho para tentar novamente.",
                )
                .await;
                return;
            }
        };
        let source = normalize(&output.text);
        self.text_captured = result.captured;
        {
            let mut shared = self.shared.lock().unwrap();
            shared.snapshot.ocr_count += 1;
            shared.snapshot.ocr_ms = output.elapsed_ms;
            shared.snapshot.ocr_text = source.clone();
            shared.snapshot.ocr_confidence = Some(output.confidence);
            shared.snapshot.ocr_confirmations = 1;
        }
        tracing::info!(
            generation = self.generation,
            ocr_ms = output.elapsed_ms,
            confidence = output.confidence,
            characters = source.chars().count(),
            "manual_ocr"
        );
        if source.is_empty() {
            self.manual_pending = false;
            self.status(
                "running",
                "Nenhum texto legível. Ajuste a área ou tente novamente pelo atalho.",
            )
            .await;
            return;
        }
        if let Some(text) = self.cache.get(&source).cloned() {
            self.shared.lock().unwrap().snapshot.cache_hits += 1;
            self.manual_pending = false;
            self.show(&text).await;
            self.status(
                "running",
                "Tradução pronta. Use o atalho para traduzir outro texto.",
            )
            .await;
            return;
        }
        if source.chars().count() > 4000 {
            self.manual_pending = false;
            self.status("running", "Texto muito longo. Selecione uma área menor.")
                .await;
            return;
        }
        let translator = self.translator.clone();
        let key = self.key.clone();
        let generation = self.generation;
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
                source,
                elapsed: start.elapsed().as_millis() as u64,
                result,
            }
        }));
        self.status("running", "Traduzindo a captura solicitada…")
            .await;
        tracing::info!(generation, "translation_requested");
    }

    async fn finish_network(&mut self) {
        if !self.network.as_ref().is_some_and(|task| task.is_finished()) {
            return;
        }
        self.shared.lock().unwrap().snapshot.api_pending = false;
        let result = self.network.take().unwrap().await;
        let Ok(result) = result else {
            self.manual_pending = false;
            self.status(
                "running",
                "Tradução interrompida. Pressione o atalho novamente.",
            )
            .await;
            return;
        };
        if result.generation != self.generation || !self.running || !self.manual_pending {
            return;
        }
        self.manual_pending = false;
        self.shared.lock().unwrap().snapshot.api_ms = result.elapsed;
        match result.result {
            Ok(text) => {
                self.shared.lock().unwrap().snapshot.api_successes += 1;
                self.cache.put(result.source, text.clone());
                self.failures = 0;
                self.retry_at = Instant::now();
                self.shared.lock().unwrap().snapshot.latency_ms =
                    self.text_captured.elapsed().as_millis() as u64;
                self.show(&text).await;
                self.status(
                    "running",
                    "Tradução pronta. Use o atalho para traduzir outro texto.",
                )
                .await;
                tracing::info!(api_ms = result.elapsed, "translation");
            }
            Err(error) => {
                if error.suspends() {
                    self.running = false;
                    if let Some(capture) = &self.capture {
                        let _ = capture.pause(true);
                    }
                    self.status("blocked", error.message()).await;
                } else {
                    self.failures = (self.failures + 1).min(5);
                    self.retry_at = Instant::now() + Duration::from_secs(1 << self.failures);
                    self.status(
                        "running",
                        "Falha na tradução. Aguarde alguns segundos e tente novamente pelo atalho.",
                    )
                    .await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn engine() -> Engine {
        assert_eq!(
            std::env::var("AREA_TRANSLATOR_ISOLATED_TEST").as_deref(),
            Ok("1")
        );
        gstreamer::init().unwrap();
        let mut engine = Engine::new(
            Arc::new(Mutex::new(crate::service::Shared::default())),
            zbus::Connection::session().await.unwrap(),
        )
        .unwrap();
        engine.running = true;
        engine.shared.lock().unwrap().frames = Some(Arc::new(Mutex::new(
            crate::capture::SharedFrames::default(),
        )));
        fixture(&mut engine, "dialog");
        engine
    }

    fn fixture(engine: &mut Engine, name: &str) {
        let mut reader = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(format!("tests/fixtures/{name}.png")).unwrap(),
        ))
        .read_info()
        .unwrap();
        let mut gray = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut gray).unwrap();
        assert_eq!(frame.color_type, png::ColorType::Grayscale);
        let info = gstreamer_video::VideoInfo::builder(
            gstreamer_video::VideoFormat::Rgbx,
            frame.width,
            frame.height,
        )
        .build()
        .unwrap();
        let mut buffer = gstreamer::Buffer::with_size(info.size()).unwrap();
        {
            let mut map = buffer.get_mut().unwrap().map_writable().unwrap();
            for (pixel, value) in map.as_mut_slice().chunks_exact_mut(4).zip(gray) {
                pixel.copy_from_slice(&[value, value, value, 255]);
            }
        }
        let sample = gstreamer::Sample::builder()
            .buffer(&buffer)
            .caps(&info.to_caps().unwrap())
            .build();
        let region = Rect {
            x: 0,
            y: 0,
            width: frame.width,
            height: frame.height,
        };
        let mut shared = engine.shared.lock().unwrap();
        shared.snapshot.region = Some(region);
        let mut slot = shared.frames.as_ref().unwrap().lock().unwrap();
        slot.dimensions = None; // Fixtures may use different synthetic dimensions.
        slot.region = Some(region);
        crate::capture::retain_sample(&sample, &mut slot).unwrap();
    }

    async fn ocr(engine: &mut Engine) {
        let result = tokio::time::timeout(Duration::from_secs(10), engine.ocr_rx.recv())
            .await
            .unwrap()
            .unwrap();
        engine.ocr_result(result).await;
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

    async fn server(engine: &mut Engine, status: u16) -> (Arc<AtomicUsize>, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        engine.translator =
            Translator::with_test_endpoint(format!("http://{}", listener.local_addr().unwrap()));
        engine.key = "test-only-no-network-to-google".into();
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![];
                loop {
                    let mut bytes = [0; 2048];
                    let n = socket.read(&mut bytes).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&bytes[..n]);
                    if let Some((headers, body)) = std::str::from_utf8(&request)
                        .unwrap()
                        .split_once("\r\n\r\n")
                    {
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        if body.len() >= length {
                            break;
                        }
                    }
                }
                requests.fetch_add(1, Ordering::SeqCst);
                let body = if status == 200 {
                    r#"{"data":{"translations":[{"translatedText":"Legenda manual."}]}}"#
                } else {
                    "{}"
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (count, task)
    }

    #[tokio::test]
    #[ignore = "native OCR; run under isolated D-Bus with AREA_TRANSLATOR_ISOLATED_TEST=1"]
    async fn only_explicit_requests_run_one_ocr_and_cache_avoids_repeat_api() {
        let mut e = engine().await;
        let (count, task) = server(&mut e, 200).await;
        for _ in 0..20 {
            e.tick().await;
        }
        assert_eq!(e.shared.lock().unwrap().snapshot.ocr_count, 0);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        e.refresh().await.unwrap();
        for _ in 0..20 {
            e.refresh().await.unwrap();
        } // Held key never queues extra work.
        assert_eq!(e.shared.lock().unwrap().snapshot.manual_requests, 1);
        ocr(&mut e).await;
        finish(&mut e).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            e.shared.lock().unwrap().snapshot.translation,
            "Legenda manual."
        );
        fixture(&mut e, "blank");
        for _ in 0..20 {
            e.tick().await;
        }
        assert_eq!(
            e.shared.lock().unwrap().snapshot.ocr_count,
            1,
            "Scene changes cannot trigger OCR"
        );
        e.refresh().await.unwrap();
        assert_eq!(
            e.shared.lock().unwrap().snapshot.translation,
            "Legenda manual.",
            "Keep subtitle while reading"
        );
        ocr(&mut e).await;
        assert!(!e.manual_pending);
        assert_eq!(
            e.shared.lock().unwrap().snapshot.translation,
            "Legenda manual.",
            "Empty reading must preserve subtitle"
        );
        fixture(&mut e, "dialog");
        e.refresh().await.unwrap();
        ocr(&mut e).await;
        assert_eq!(e.shared.lock().unwrap().snapshot.cache_hits, 1);
        for _ in 0..20 {
            e.tick().await;
        }
        assert_eq!(e.shared.lock().unwrap().snapshot.ocr_count, 3);
        assert_eq!(e.shared.lock().unwrap().snapshot.manual_requests, 3);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(!e.ocr_busy && !e.manual_pending && e.network.is_none());
        e.stop().await;
        task.abort();
    }

    #[tokio::test]
    #[ignore = "native OCR; run under isolated D-Bus with AREA_TRANSLATOR_ISOLATED_TEST=1"]
    async fn api_errors_never_retry_without_another_keypress() {
        for status in [503, 403, 429] {
            let mut e = engine().await;
            e.show("Legenda anterior.").await;
            let (count, task) = server(&mut e, status).await;
            e.refresh().await.unwrap();
            ocr(&mut e).await;
            finish(&mut e).await;
            assert_eq!(
                e.shared.lock().unwrap().snapshot.translation,
                "Legenda anterior."
            );
            assert!(!e.manual_pending);
            if status == 503 {
                assert!(
                    e.refresh().await.is_err(),
                    "Backoff must apply to manual retries"
                );
            }
            e.retry_at = Instant::now();
            for _ in 0..20 {
                e.tick().await;
            }
            assert_eq!(count.load(Ordering::SeqCst), 1);
            if status == 503 {
                e.refresh().await.unwrap();
                ocr(&mut e).await;
                finish(&mut e).await;
                assert_eq!(count.load(Ordering::SeqCst), 2);
            } else {
                assert!(e.refresh().await.is_err());
                assert_eq!(e.shared.lock().unwrap().snapshot.state, "blocked");
            }
            e.stop().await;
            task.abort();
        }
    }

    #[tokio::test]
    #[ignore = "native OCR; run under isolated D-Bus with AREA_TRANSLATOR_ISOLATED_TEST=1"]
    async fn pause_and_stop_reject_in_flight_ocr_and_disallow_shortcuts() {
        let mut e = engine().await;
        e.refresh().await.unwrap();
        let (reply, receive) = oneshot::channel();
        e.command(Command::Pause(reply)).await;
        receive.await.unwrap().unwrap();
        assert!(e.refresh().await.is_err());
        ocr(&mut e).await;
        assert_eq!(e.shared.lock().unwrap().snapshot.ocr_count, 0);
        assert!(e.network.is_none());
        e.stop().await;
        assert!(e.refresh().await.is_err());
        assert!(e.pending_frame.is_none());
    }

    #[tokio::test]
    #[ignore = "run under isolated D-Bus with AREA_TRANSLATOR_ISOLATED_TEST=1"]
    async fn pause_aborts_network_and_new_generation_rejects_old_response() {
        let mut e = engine().await;
        e.manual_pending = true;
        let (reply, response) = oneshot::channel();
        e.network = Some(tokio::spawn(async move { response.await.unwrap() }));
        let (command_reply, command_response) = oneshot::channel();
        e.command(Command::Pause(command_reply)).await;
        command_response.await.unwrap().unwrap();
        tokio::task::yield_now().await;
        assert!(reply.is_closed());
        assert!(e.network.is_none());
        e.running = true;
        e.manual_pending = true;
        let generation = e.generation - 1;
        e.network = Some(tokio::spawn(async move {
            NetworkResult {
                generation,
                source: "old".into(),
                elapsed: 1,
                result: Ok("Old response".into()),
            }
        }));
        finish(&mut e).await;
        assert!(e.shared.lock().unwrap().snapshot.translation.is_empty());
        assert_eq!(e.shared.lock().unwrap().snapshot.api_successes, 0);
        e.stop().await;
    }
}
