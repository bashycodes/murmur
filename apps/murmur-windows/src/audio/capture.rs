//! cpal (WASAPI shared-mode) microphone capture → 16 kHz mono i16 sink.
//!
//! cpal `Stream` is `!Send`, so each capture owns a thread that builds the
//! stream, plays it and parks until stopped. The sink runs on cpal's audio
//! callback thread: keep it cheap (`CoreApi::push_audio` is non-blocking by
//! contract, the level is an atomic store).

use super::Resampler;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::sync::mpsc;
use std::thread::JoinHandle;

pub type Sink = Box<dyn FnMut(&[i16]) + Send + 'static>;

pub struct Capture {
    stop: Option<mpsc::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

pub fn list_input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn pick_device(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Some(n) = name {
        if let Ok(mut it) = host.input_devices() {
            if let Some(d) = it.find(|d| d.name().ok().as_deref() == Some(n)) {
                return Ok(d);
            }
        }
    }
    host.default_input_device()
        .ok_or_else(|| "no input device".to_string())
}

/// Called (from the audio thread) when the stream fails mid-capture, e.g.
/// the mic was unplugged (review P3: a hands-free session must not keep
/// "listening" to nothing).
pub type OnError = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// Which side of a device to capture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// A microphone.
    Input,
    /// What an output device plays (WASAPI loopback: cpal turns it on for
    /// an input stream on a render device). No callbacks while nothing
    /// plays; the meeting recorder fills those gaps.
    Loopback,
}

fn build(
    device: &cpal::Device,
    side: Side,
    mut sink: Sink,
    on_error: OnError,
) -> Result<cpal::Stream, String> {
    let supported = match side {
        Side::Input => device.default_input_config(),
        Side::Loopback => device.default_output_config(),
    }
    .map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let mut rs = Resampler::new(config.sample_rate.0, config.channels);
    let err_fn = move |e: cpal::StreamError| {
        eprintln!("[audio] stream error: {e}");
        crate::debug_log::event("audio", &format!("stream error: {e}"));
        on_error(match side {
            Side::Input => format!("Microphone stopped working ({e})"),
            Side::Loopback => format!("System audio stopped recording ({e})"),
        });
    };
    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &_| sink(&rs.process(data)),
            err_fn,
            None,
        ),
        SampleFormat::I16 => {
            let mut buf = Vec::new();
            device.build_input_stream(
                &config,
                move |data: &[i16], _: &_| {
                    buf.clear();
                    buf.extend(data.iter().map(|s| *s as f32 / 32768.0));
                    sink(&rs.process(&buf));
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let mut buf = Vec::new();
            device.build_input_stream(
                &config,
                move |data: &[u16], _: &_| {
                    buf.clear();
                    buf.extend(data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0));
                    sink(&rs.process(&buf));
                },
                err_fn,
                None,
            )
        }
        other => return Err(format!("unsupported sample format {other:?}")),
    };
    stream.map_err(|e| e.to_string())
}

fn default_output() -> Result<cpal::Device, String> {
    cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "no playback device".to_string())
}

impl Capture {
    /// Start capturing from `device` (by name; `None` = system default).
    pub fn start(device: Option<String>, sink: Sink, on_error: OnError) -> Result<Capture, String> {
        Self::spawn("murmur-audio", sink, on_error, move || {
            pick_device(device.as_deref()).map(|d| (d, Side::Input))
        })
    }

    /// Capture what the default playback device plays (WASAPI loopback):
    /// the other side of a call, for the Notetaker (D45).
    pub fn start_loopback(sink: Sink, on_error: OnError) -> Result<Capture, String> {
        Self::spawn("murmur-loopback", sink, on_error, || {
            default_output().map(|d| (d, Side::Loopback))
        })
    }

    fn spawn(
        name: &str,
        sink: Sink,
        on_error: OnError,
        device: impl FnOnce() -> Result<(cpal::Device, Side), String> + Send + 'static,
    ) -> Result<Capture, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let join = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let res = device().and_then(|(d, side)| build(&d, side, sink, on_error));
                match res {
                    Ok(stream) => {
                        if let Err(e) = stream.play() {
                            let _ = ready_tx.send(Err(e.to_string()));
                            return;
                        }
                        let _ = ready_tx.send(Ok(()));
                        let _ = stop_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|e| e.to_string())??;
        Ok(Capture {
            stop: Some(stop_tx),
            join: Some(join),
        })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shutdown();
    }
}
