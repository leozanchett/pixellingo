use area_translator::{
    engine::Engine,
    service::{BUS, PATH, Service, Shared},
};
use std::sync::{Arc, Mutex};

fn main() -> anyhow::Result<()> {
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
        println!(
            "PipeWire/GStreamer disponíveis. OCR requer Cloud Vision e credencial configurada."
        );
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
            // Installed companion watches this unique owner, including unexpected exits.
            // It reserves the configured key only while capture is running, without GTK.
            let executable = std::env::current_exe()?;
            let guard = executable
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("ui/shortcut-guard.js"));
            if let Some(guard) = guard.filter(|p| p.is_file()) {
                let child = std::process::Command::new("gjs")
                    .arg("-m")
                    .arg(guard)
                    .arg(
                        connection
                            .unique_name()
                            .expect("session bus unique name")
                            .as_str(),
                    )
                    .spawn()?;
                // Reap the companion when it observes our connection closing.
                std::thread::spawn(move || {
                    let mut child = child;
                    let _ = child.wait();
                });
            }
            Engine::new(shared, connection)?.run(receiver).await;
            Ok(())
        })
}
