use area_translator::{
    engine::Engine,
    service::{BUS, PATH, Service, Shared},
};
use std::sync::{Arc, Mutex};

fn main() -> anyhow::Result<()> {
    // SAFETY: before starting the runtime or any native library threads.
    unsafe {
        std::env::set_var("OMP_THREAD_LIMIT", "1");
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "area_translator=info".into()),
        )
        .init();
    gstreamer::init()?;
    if std::env::args().any(|a| a == "--check") {
        anyhow::ensure!(
            gstreamer::ElementFactory::find("pipewiresrc").is_some(),
            "Instale gstreamer1.0-pipewire."
        );
        let _ocr = area_translator::ocr::Ocr::new()?;
        println!("PipeWire/GStreamer e OCR inglês disponíveis.");
        return Ok(());
    }
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(async {
            let (commands, receiver) = tokio::sync::mpsc::channel(16);
            let shared = Arc::new(Mutex::new(Shared::default()));
            let connection = zbus::connection::Builder::session()?
                .name(BUS)?
                .serve_at(
                    PATH,
                    Service {
                        commands,
                        shared: shared.clone(),
                    },
                )?
                .build()
                .await?;
            Engine::new(shared, connection)?.run(receiver).await;
            Ok(())
        })
}
