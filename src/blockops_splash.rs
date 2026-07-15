use std::time::{Duration, Instant};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, RgbaImage};

pub struct BlockOpsSplash {
    started: Instant,
    frames: Vec<TextureHandle>,
    frame_delays: Vec<Duration>,
    total_duration: Duration,
    loaded: bool,
}

impl Default for BlockOpsSplash {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            frames: Vec::new(),
            frame_delays: Vec::new(),
            total_duration: Duration::from_millis(4500),
            loaded: false,
        }
    }
}

impl BlockOpsSplash {
    pub fn load(&mut self, ctx: &Context) {
        if self.loaded {
            return;
        }

        self.loaded = true;

        let bytes = include_bytes!("../assets/blockops_intro.gif");
        let cursor = std::io::Cursor::new(bytes);

        let Ok(decoder) = GifDecoder::new(cursor) else {
            return;
        };

        let Ok(frames) = decoder.into_frames().collect_frames() else {
            return;
        };

        let mut total_ms: u64 = 0;

        for (idx, frame) in frames.into_iter().enumerate() {
            let delay = frame.delay();
            let (num, denom) = delay.numer_denom_ms();
            let mut ms = if denom == 0 {
                40
            } else {
                (num as u64).saturating_div(denom as u64).max(16)
            };
            if ms == 0 {
                ms = 40;
            }

            let rgba: RgbaImage = frame.into_buffer();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = rgba.into_raw();
            let color_image = ColorImage::from_rgba_unmultiplied(size, &pixels);
            let texture = ctx.load_texture(
                format!("blockops_intro_frame_{}", idx),
                color_image,
                TextureOptions::LINEAR,
            );

            self.frames.push(texture);
            self.frame_delays.push(Duration::from_millis(ms));
            total_ms += ms;
        }

        if total_ms > 0 {
            self.total_duration = Duration::from_millis(total_ms.min(20000).max(3500));
        }
    }

    pub fn is_done(&self) -> bool {
        self.started.elapsed() >= self.total_duration
    }

    pub fn show(&mut self, ctx: &Context) {
        self.load(ctx);

        if self.frames.is_empty() {
            return;
        }

        const FIRST_FRAME_HOLD_MS: u64 = 1500;

        let elapsed_ms = self.started.elapsed().as_millis() as u64;

        // Hold the first frame first, then play through the GIF once,
        // then HOLD the final frame. No looping/replay.
        let gif_duration_ms: u64 = self
            .frame_delays
            .iter()
            .map(|d| d.as_millis().max(16) as u64)
            .sum::<u64>()
            .max(1);

        let mut frame_idx = self.frames.len().saturating_sub(1);

        if elapsed_ms < FIRST_FRAME_HOLD_MS {
            frame_idx = 0;
        } else {
            let playhead_ms = (elapsed_ms - FIRST_FRAME_HOLD_MS).min(gif_duration_ms);

            let mut acc = 0u64;

            for (idx, delay) in self.frame_delays.iter().enumerate() {
                acc += delay.as_millis().max(16) as u64;
                if playhead_ms <= acc {
                    frame_idx = idx;
                    break;
                }
            }
        }

        let texture = &self.frames[frame_idx];
        let screen = ctx.screen_rect();

        egui::Area::new(egui::Id::new("blockops_intro_splash"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let painter = ui.painter();
                painter.rect_filled(screen, 0.0, egui::Color32::from_rgb(7, 18, 60));

                // Contain/letterbox mode: show the entire intro frame.
                // This prevents the video/GIF from being cropped in the smaller
                // startup window before the app maximizes.
                let tex_size = texture.size_vec2();
                let scale = (screen.width() / tex_size.x).min(screen.height() / tex_size.y);
                let drawn_size = tex_size * scale;
                let pos = screen.center() - drawn_size / 2.0;
                let rect = egui::Rect::from_min_size(pos, drawn_size);

                painter.image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::new(0.0, 0.0), egui::Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            });

        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
