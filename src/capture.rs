use crate::model::{CaptureSource, Frame, Rect, SCAN_INTERVAL};
use anyhow::{Context, Result};
use ashpd::desktop::{
    Session,
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
};
use futures_util::StreamExt;
use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;
use gstreamer_video::{self as gst_video, prelude::*};
use std::{
    os::fd::{AsRawFd, OwnedFd},
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Default)]
pub struct SharedFrames {
    pub region: Option<Rect>,
    pub frame: Option<Frame>,
    pub preview: Option<(u32, u32, Vec<u8>)>,
    pub dimensions: Option<(u32, u32)>,
    pub error: Option<String>,
    pub paused: bool,
    last_sample: Option<Instant>,
}

pub type FrameSlot = Arc<Mutex<SharedFrames>>;

pub struct Capture {
    pipeline: gst::Pipeline,
    session: Arc<Session<Screencast>>,
    closed_listener: tokio::task::JoinHandle<()>,
    // The remote connection must live at least as long as pipewiresrc.
    _fd: OwnedFd,
    pub slot: FrameSlot,
    pub position: Option<(i32, i32)>,
    pub logical_size: Option<(i32, i32)>,
    pub source: CaptureSource,
}

impl Capture {
    pub async fn start(
        source_kind: CaptureSource,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<Self> {
        let portal = Screencast::new().await?;
        let source_type = match source_kind {
            CaptureSource::Monitor => SourceType::Monitor,
            CaptureSource::Window => SourceType::Window,
        };
        anyhow::ensure!(
            portal.available_source_types().await?.contains(source_type),
            "O sistema não oferece este tipo de captura."
        );
        let session = portal.create_session(Default::default()).await?;
        let open = async {
            portal
                .select_sources(
                    &session,
                    SelectSourcesOptions::default()
                        .set_sources(Some(source_type.into()))
                        .set_multiple(false)
                        .set_cursor_mode(CursorMode::Hidden)
                        .set_persist_mode(ashpd::desktop::PersistMode::DoNot),
                )
                .await?;
            let response = portal
                .start(&session, None, Default::default())
                .await?
                .response()?;
            let stream = response
                .streams()
                .first()
                .context("Nenhuma fonte de captura selecionada.")?;
            anyhow::ensure!(
                stream
                    .source_type()
                    .is_none_or(|actual| actual == source_type),
                "O portal retornou um tipo de captura diferente do solicitado."
            );
            let fd = portal
                .open_pipe_wire_remote(&session, Default::default())
                .await?;
            let slot = Arc::new(Mutex::new(SharedFrames::default()));
            let source = gst::ElementFactory::make("pipewiresrc")
                .property("fd", fd.as_raw_fd())
                .property("path", stream.pipe_wire_node_id().to_string())
                .build()
                .context("Instale gstreamer1.0-pipewire.")?;
            let caps = gst::Caps::builder("video/x-raw")
                .field("format", gst::List::new(["BGRx", "RGBx", "BGRA", "RGBA"]))
                .build();
            let sink = gst_app::AppSink::builder()
                .caps(&caps)
                .max_buffers(1)
                .drop(true)
                .sync(false)
                .enable_last_sample(false)
                .build();
            let callback_slot = slot.clone();
            sink.set_callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        let mut state = callback_slot.lock().unwrap();
                        let now = Instant::now();
                        // Throttle BEFORE mapping/converting. No full-frame videoconvert.
                        if state.paused
                            || state
                                .last_sample
                                .is_some_and(|t| now.duration_since(t) < SCAN_INTERVAL)
                        {
                            return Ok(gst::FlowSuccess::Ok);
                        }
                        state.last_sample = Some(now);
                        if let Err(error) = consume_sample(&sample, &mut state) {
                            state.error = Some(error.to_string());
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
            let pipeline = gst::Pipeline::default();
            pipeline.add_many([&source, sink.upcast_ref()])?;
            source.link(&sink)?;
            if let Err(error) = pipeline.set_state(gst::State::Playing) {
                let _ = pipeline.set_state(gst::State::Null);
                return Err(error.into());
            }
            Ok::<_, anyhow::Error>((pipeline, fd, slot, stream.position(), stream.size()))
        };
        let result = tokio::select! {
            result = open => result,
            _ = &mut cancel => Err(anyhow::anyhow!("Seleção cancelada.")),
        };
        match result {
            Ok((pipeline, fd, slot, position, logical_size)) => {
                let session = Arc::new(session);
                let listener_session = session.clone();
                let listener_slot = slot.clone();
                let closed_listener = tokio::spawn(async move {
                    if let Ok(mut stream) = listener_session.receive_closed().await {
                        stream.next().await;
                        listener_slot.lock().unwrap().error =
                            Some("O compartilhamento da tela foi encerrado.".into());
                    }
                });
                Ok(Self {
                    pipeline,
                    session,
                    closed_listener,
                    _fd: fd,
                    slot,
                    position,
                    logical_size,
                    source: source_kind,
                })
            }
            Err(error) => {
                let _ = session.close().await;
                Err(error)
            }
        }
    }

    pub fn pause(&self, paused: bool) -> Result<()> {
        let mut state = self.slot.lock().unwrap();
        state.paused = paused;
        state.frame = None;
        state.last_sample = None;
        drop(state);
        self.pipeline.set_state(if paused {
            gst::State::Paused
        } else {
            gst::State::Playing
        })?;
        Ok(())
    }

    pub fn error(&self) -> Option<String> {
        if let Some(error) = self.slot.lock().unwrap().error.take() {
            return Some(error);
        }
        let bus = self.pipeline.bus()?;
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Error(e) => {
                    return Some(format!("Captura interrompida: {}", e.error()));
                }
                gst::MessageView::Eos(_) => {
                    return Some("O compartilhamento da tela foi encerrado.".into());
                }
                _ => {}
            }
        }
        None
    }

    pub async fn close(self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = self.session.close().await;
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.closed_listener.abort();
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn consume_sample(sample: &gst::Sample, state: &mut SharedFrames) -> Result<()> {
    let info = gst_video::VideoInfo::from_caps(sample.caps().context("Captura sem formato.")?)?;
    let dimensions = (info.width(), info.height());
    anyhow::ensure!(
        dimensions.0 > 0
            && dimensions.1 > 0
            && dimensions.0 as u64 * dimensions.1 as u64 <= 34_000_000,
        "Resolução de captura não suportada."
    );
    if state.dimensions.is_some_and(|old| old != dimensions) {
        anyhow::bail!("O tamanho da captura mudou. Selecione a área novamente.");
    }
    state.dimensions = Some(dimensions);
    // During selection retain a single frozen preview, not a stream of full images.
    if state.region.is_none() && state.preview.is_some() {
        return Ok(());
    }
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(
        sample.buffer().context("Quadro vazio.")?,
        &info,
    )?;
    let data = frame.plane_data(0)?;
    let stride = frame.plane_stride()[0];
    anyhow::ensure!(stride > 0, "Stride de captura não suportado.");
    let stride = stride as usize;
    let bgr = matches!(
        info.format(),
        gst_video::VideoFormat::Bgrx | gst_video::VideoFormat::Bgra
    );
    let region = state.region.unwrap_or(Rect {
        x: 0,
        y: 0,
        width: dimensions.0,
        height: dimensions.1,
    });
    if state.region.is_some() {
        region.validate(dimensions.0, dimensions.1)?;
    }
    let mut pixels = Vec::with_capacity(
        region.width as usize * region.height as usize * if state.region.is_none() { 3 } else { 1 },
    );
    for y in region.y..region.y + region.height {
        let start = y as usize * stride + region.x as usize * 4;
        let row = data
            .get(start..start + region.width as usize * 4)
            .context("Buffer de captura incompleto.")?;
        for p in row.chunks_exact(4) {
            let (r, g, b) = if bgr {
                (p[2], p[1], p[0])
            } else {
                (p[0], p[1], p[2])
            };
            if state.region.is_none() {
                pixels.extend_from_slice(&[r, g, b]);
            } else {
                pixels.push(((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8);
            }
        }
    }
    if state.region.is_none() {
        state.preview = Some((region.width, region.height, pixels));
    } else {
        state.frame = Some(Frame {
            width: region.width,
            height: region.height,
            gray: pixels,
            captured: Instant::now(),
        });
    }
    Ok(())
}

pub fn preview_png(slot: &FrameSlot) -> Result<Vec<u8>> {
    let state = slot.lock().unwrap();
    let (width, height, pixels) = state
        .preview
        .as_ref()
        .context("Aguardando primeiro quadro…")?;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, *width, *height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.write_header()?.write_image_data(pixels)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_uses_row_stride_and_bgr_channel_order() {
        gst::init().unwrap();
        let info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Bgrx, 40, 32)
            .build()
            .unwrap();
        let mut buffer = gst::Buffer::with_size(info.size()).unwrap();
        {
            let mut map = buffer.get_mut().unwrap().map_writable().unwrap();
            map.as_mut_slice().fill(0);
            for y in 4..20 {
                for x in 8..24 {
                    let i = y * info.stride()[0] as usize + x * 4;
                    map.as_mut_slice()[i..i + 4].copy_from_slice(&[0, 0, 255, 255]);
                }
            }
        }
        let sample = gst::Sample::builder()
            .buffer(&buffer)
            .caps(&info.to_caps().unwrap())
            .build();
        let mut state = SharedFrames {
            region: Some(Rect {
                x: 8,
                y: 4,
                width: 16,
                height: 16,
            }),
            ..Default::default()
        };
        consume_sample(&sample, &mut state).unwrap();
        let frame = state.frame.unwrap();
        assert_eq!(frame.gray.len(), 256);
        assert!(frame.gray.iter().all(|p| *p == 76));
    }

    #[test]
    fn resolution_changes_require_reselection() {
        gst::init().unwrap();
        let info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgbx, 100, 100)
            .build()
            .unwrap();
        let sample = gst::Sample::builder()
            .caps(&info.to_caps().unwrap())
            .build();
        let mut state = SharedFrames {
            dimensions: Some((200, 200)),
            ..Default::default()
        };
        assert!(
            consume_sample(&sample, &mut state)
                .unwrap_err()
                .to_string()
                .contains("tamanho da captura mudou")
        );
    }
}
