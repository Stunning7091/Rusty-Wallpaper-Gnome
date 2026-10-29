use std::cell::Cell;
use std::path::Path;

use gst::prelude::*;
use gtk::{gdk, glib};

/// One decoded video loop. Monitors showing the same file share a `Video`,
/// so the file is only decoded once.
pub struct Video {
    name: String,
    pipeline: gst::Element,
    /// The state last asked for; the pipeline may still be changing to it.
    playing: Cell<bool>,
    paintable: gdk::Paintable,
    _bus_watch: gst::bus::BusWatchGuard,
}

impl Video {
    pub fn new(file: &Path) -> Result<Self, String> {
        let sink = gst::ElementFactory::make("gtk4paintablesink")
            .build()
            .map_err(|e| format!("gtk4paintablesink: {e}"))?;
        let paintable = sink.property::<gdk::Paintable>("paintable");

        let uri =
            glib::filename_to_uri(file, None).map_err(|e| format!("{}: {e}", file.display()))?;
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri.as_str())
            .property("video-sink", &sink)
            .build()
            .map_err(|e| format!("playbin: {e}"))?;
        // Video only: audio and subtitle streams are never decoded.
        pipeline.set_property_from_str("flags", "video");

        let name = file.display().to_string();
        let watch_name = name.clone();
        let looping = Cell::new(false);
        let weak = pipeline.downgrade();
        let bus_watch = pipeline
            .bus()
            .expect("playbin has a bus")
            .add_watch_local(move |_, msg| {
                let Some(pipeline) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                match msg.view() {
                    // Once prerolled, switch to segment seeks: they loop without
                    // flushing, so there is no gap or black frame at the seam.
                    gst::MessageView::AsyncDone(_) if !looping.get() => {
                        looping.set(true);
                        eprintln!(
                            "rusty-wallpaper: {watch_name}: decoding with {}",
                            decoder_name(&pipeline)
                        );
                        seek_to_start(&pipeline, gst::SeekFlags::FLUSH);
                    }
                    gst::MessageView::SegmentDone(_) => {
                        seek_to_start(&pipeline, gst::SeekFlags::empty());
                    }
                    // Fallback for streams that end without honouring the segment seek.
                    gst::MessageView::Eos(_) => seek_to_start(&pipeline, gst::SeekFlags::FLUSH),
                    gst::MessageView::Error(err) => {
                        eprintln!(
                            "rusty-wallpaper: {watch_name}: {} ({:?})",
                            err.error(),
                            err.debug()
                        );
                    }
                    _ => {}
                }
                glib::ControlFlow::Continue
            })
            .map_err(|e| format!("bus watch: {e}"))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("{}: cannot play: {e}", file.display()))?;

        Ok(Self {
            name,
            pipeline,
            playing: Cell::new(true),
            paintable,
            _bus_watch: bus_watch,
        })
    }

    pub fn paintable(&self) -> &gdk::Paintable {
        &self.paintable
    }

    pub fn set_playing(&self, playing: bool) {
        if self.playing.replace(playing) == playing {
            return;
        }
        let state = if playing {
            gst::State::Playing
        } else {
            gst::State::Paused
        };
        if std::env::var_os("RUSTY_WALLPAPER_DEBUG").is_some() {
            eprintln!("rusty-wallpaper: {}: {state:?}", self.name);
        }
        let _ = self.pipeline.set_state(state);
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn seek_to_start(pipeline: &gst::Element, flags: gst::SeekFlags) {
    let result = pipeline.seek(
        1.0,
        flags | gst::SeekFlags::SEGMENT,
        gst::SeekType::Set,
        gst::ClockTime::ZERO,
        gst::SeekType::None,
        gst::ClockTime::NONE,
    );
    if let Err(e) = result {
        eprintln!("rusty-wallpaper: loop seek failed: {e}");
    }
}

fn decoder_name(pipeline: &gst::Element) -> String {
    let bin = pipeline
        .downcast_ref::<gst::Bin>()
        .expect("playbin is a bin");
    bin.iterate_recurse()
        .into_iter()
        .flatten()
        .filter_map(|element| element.factory())
        .find(|factory| {
            let klass = factory.klass();
            klass.contains("Decoder") && klass.contains("Video")
        })
        .map_or_else(
            || "an unknown decoder".to_owned(),
            |factory| factory.name().to_string(),
        )
}

/// Ranks the VA-API decoders above the software ones so playbin picks them.
pub fn prefer_hardware_decoders() {
    let registry = gst::Registry::get();
    for feature in registry.features_by_plugin("va") {
        if feature.name().ends_with("dec") {
            feature.set_rank(gst::Rank::PRIMARY + 1);
        }
    }
}
