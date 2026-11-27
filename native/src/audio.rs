//! In-process GStreamer audio pipeline. Remote providers are resolved by HomeMusic.
use gstreamer::{self as gst, prelude::*};
use std::{
    cell::Cell,
    sync::{Arc, Mutex},
};
pub struct Player {
    pipeline: gst::Element,
    bus: gst::Bus,
    cookie: Arc<Mutex<String>>,
    paused: Cell<bool>,
    idle: Cell<bool>,
    ended: Cell<bool>,
    failed: Cell<bool>,
}
impl Player {
    pub fn new() -> Result<Self, String> {
        gst::init().map_err(|e| e.to_string())?;
        let pipeline = gst::ElementFactory::make("playbin3")
            .build()
            .map_err(|e| e.to_string())?;
        // Audio-only, bounded network buffering; no video decoder or external player process.
        pipeline.set_property_from_str("flags", "audio+buffering");
        pipeline.set_property("buffer-duration", 8_000_000_000i64);
        pipeline.set_property("buffer-size", 2_097_152i32);
        pipeline.set_property("volume", 0.04f64);
        let cookie = Arc::new(Mutex::new(String::new()));
        let header = cookie.clone();
        pipeline.connect("source-setup", false, move |values| {
            if let Ok(source) = values[1].get::<gst::Element>() {
                if source.find_property("cookies").is_some() {
                    let cookie = header.lock().unwrap().clone();
                    source.set_property("cookies", gst::glib::StrV::from(vec![cookie]));
                }
                if source.find_property("extra-headers").is_some() {
                    let c = header.lock().unwrap().clone();
                    source.set_property(
                        "extra-headers",
                        gst::Structure::builder("headers")
                            .field("Cookie", c)
                            .build(),
                    );
                }
                if source.find_property("proxy").is_some() {
                    source.set_property("proxy", "");
                }
            }
            None
        });
        let bus = pipeline.bus().ok_or("Нет шины GStreamer")?;
        Ok(Self {
            pipeline,
            bus,
            cookie,
            paused: Cell::new(false),
            idle: Cell::new(true),
            ended: Cell::new(false),
            failed: Cell::new(false),
        })
    }
    fn poll(&self) {
        for message in self.bus.iter() {
            match message.view() {
                gst::MessageView::Eos(..) => {
                    self.ended.set(true);
                    self.idle.set(true);
                }
                gst::MessageView::Error(e) => {
                    eprintln!("Audio pipeline: {}", e.error());
                    self.idle.set(true);
                    self.failed.set(true);
                }
                _ => (),
            }
        }
    }
    pub fn set(&self, key: &str, value: &str) {
        match key {
            "volume" => self.pipeline.set_property(
                "volume",
                (value.parse::<f64>().unwrap_or(20.).clamp(0., 100.) / 100.).powi(2),
            ),
            "mute" => self.pipeline.set_property("mute", value == "yes"),
            "pause" => {
                let pause = value == "yes";
                self.paused.set(pause);
                let _ = self.pipeline.set_state(if pause {
                    gst::State::Paused
                } else {
                    gst::State::Playing
                });
            }
            _ => (),
        }
    }
    pub fn number(&self, key: &str) -> f64 {
        self.poll();
        match key {
            "time-pos" => self.position().unwrap_or(0.),
            "duration" => self
                .pipeline
                .query_duration::<gst::ClockTime>()
                .map(|v| v.nseconds() as f64 / 1e9)
                .unwrap_or(0.),
            "volume" => self.pipeline.property::<f64>("volume").sqrt() * 100.,
            _ => 0.,
        }
    }
    pub fn position(&self) -> Option<f64> {
        self.pipeline.query_position::<gst::ClockTime>()
            .map(|value| value.nseconds() as f64 / 1e9)
    }
    pub fn flag(&self, key: &str) -> bool {
        self.poll();
        match key {
            "pause" => self.paused.get(),
            "idle-active" => self.idle.get(),
            _ => false,
        }
    }
    pub fn take_end(&self) -> bool {
        self.poll();
        self.ended.replace(false)
    }
    pub fn take_failure(&self) -> bool {
        self.poll();
        self.failed.replace(false)
    }
    pub fn load(&self, url: &str, cookie: &str, paused: bool, buffer: u64) -> bool {
        if !url.starts_with(&format!("{}/", crate::config::backend())) {
            return false;
        }
        let _ = self.pipeline.set_state(gst::State::Null);
        let (seconds, bytes) = match buffer {
            1 => (3_i64, 1_048_576_i32),
            2 => (16, 4_194_304),
            _ => (8, 2_097_152),
        };
        self.pipeline.set_property("buffer-duration", seconds * 1_000_000_000);
        self.pipeline.set_property("buffer-size", bytes);
        *self.cookie.lock().unwrap() = cookie.into();
        self.pipeline.set_property("uri", url);
        self.ended.set(false);
        self.paused.set(paused);
        self.failed.set(false);
        let loaded = self.pipeline
            .set_state(if paused {
                gst::State::Paused
            } else {
                gst::State::Playing
            })
            .is_ok();
        self.idle.set(!loaded);
        loaded
    }
    pub fn command(&self, args: &[&str]) -> bool {
        match args.first().copied() {
            Some("stop") => {
                let _ = self.pipeline.set_state(gst::State::Null);
                self.idle.set(true);
                self.ended.set(false);
                self.failed.set(false);
                true
            }
            Some("seek") => {
                let seconds = args
                    .get(1)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.)
                    .max(0.);
                self.ended.set(false);
                self.pipeline
                    .seek_simple(
                        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                        gst::ClockTime::from_nseconds((seconds * 1e9) as u64),
                    )
                    .is_ok()
            }
            Some("cycle") => {
                self.toggle();
                true
            }
            _ => false,
        }
    }
    pub fn toggle(&self) {
        self.set("pause", if self.paused.get() { "no" } else { "yes" });
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
