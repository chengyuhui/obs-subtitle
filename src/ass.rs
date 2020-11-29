use anyhow::Result;
use libass::{Change, DefaultFontProvider, Layer, Library, Renderer, Track};
use log::info;
use log::trace;
use obs_wrapper::graphics::{GraphicsTexture, MappedTexture};
use packed_simd_2::{u16x4, u8x4, FromCast};
use std::{error::Error, fmt::Display, fs, sync::RwLock};
use subparse::{SsaFile, SubtitleFile};

struct LastLayer {
    width: usize,
    height: usize,
    x: usize,
    y: usize,
}

impl LastLayer {
    fn from_layer(layer: &Layer) -> Self {
        Self {
            width: layer.width as usize,
            height: layer.height as usize,
            x: layer.x as usize,
            y: layer.y as usize,
        }
    }
}

struct LoadedTrack {
    pub track: Track<'static>,
    pub len: i64,
}

impl LoadedTrack {
    fn new(track: Track<'static>, len: i64) -> Self {
        Self { track, len }
    }
}

pub struct AssData {
    renderer: Renderer<'static>,
    lib: Box<Library<'static>>,

    track: RwLock<Option<LoadedTrack>>,
    cur_time: i64,

    last_image: Vec<LastLayer>,
}

impl AssData {
    pub fn new() -> Result<Self> {
        let lib = Box::into_raw(Box::new(Library::new()?));
        let lib_ref = unsafe { lib.as_ref().unwrap() };
        let mut renderer = lib_ref.new_renderer()?;

        renderer.set_frame_size(1920, 1080);
        renderer.set_fonts(
            None,
            "sans-serif",
            DefaultFontProvider::Autodetect,
            None,
            false,
        );

        Ok(Self {
            renderer,
            lib: unsafe { Box::from_raw(lib) },

            track: RwLock::new(None),
            cur_time: 0,

            last_image: Vec::with_capacity(4),
        })
    }

    pub fn tick(&mut self, msecs: i64, tex: &mut GraphicsTexture) {
        if self.track.read().unwrap().is_none() {
            return;
        }
        self.cur_time = self.cur_time.overflowing_add(msecs).0;
        self.render(tex);
    }

    fn render(&mut self, dst: &mut GraphicsTexture) {
        let dst_w = 1920;

        let (image, change) = {
            let mut track_guard = self.track.write().unwrap();
            let track = track_guard.as_mut().unwrap();
            self.renderer.render_frame(&mut track.track, self.cur_time)
        };
        if change == Change::None {
            return;
        }
        info!("New frame, canvas cleared");

        let mut map = dst.map().unwrap();

        clear_last(&self.last_image, &mut map);
        self.last_image.clear();

        if let Some(image) = image {
            let mut cnt = 0u64;
            for layer in image {
                draw_layer(&layer, &mut map, dst_w);
                cnt += 1;
                self.last_image.push(LastLayer::from_layer(&layer));
            }
            trace!("Draw {} layers", cnt);
        }
    }

    fn load_file(path: &str) -> Result<(Vec<u8>, i64)> {
        let file_bytes = fs::read(path)?;
        let file_str = String::from_utf8_lossy(&file_bytes);

        let parsed: SubtitleFile = SsaFile::parse(&file_str)
            .map_err(SubtitleParseError::new)?
            .into();

        let entries = parsed
            .get_subtitle_entries()
            .map_err(SubtitleParseError::new)?;
        let len = entries
            .iter()
            .max_by_key(|ent| ent.timespan.end)
            .map(|ent| ent.timespan.end.msecs());

        Ok((file_bytes, len.unwrap_or(0)))
    }

    /// Loads a new track.
    pub fn load_track(&mut self, path: &str) -> Result<()> {
        let (file_bytes, track_len) = Self::load_file(path)?;
        let track = self.lib_ref().new_track_from_memory(&file_bytes, "UTF-8")?;
        let l_track = LoadedTrack::new(track, track_len);
        info!("Loaded file {}, length = {} ms.", path, l_track.len);

        self.track.write().unwrap().replace(l_track);
        self.cur_time = 0;
        Ok(())
    }

    pub fn current_len(&self) -> i64 {
        self.track
            .read()
            .unwrap()
            .as_ref()
            .map(|t| t.len)
            .unwrap_or(0)
    }

    pub fn current_time(&self) -> i64 {
        self.cur_time
    }

    pub fn ended(&self) -> bool {
        self.cur_time >= self.current_len()
    }

    pub fn loaded(&self) -> bool {
        self.track.read().unwrap().is_some()
    }

    fn lib_ref(&self) -> &'static Library<'static> {
        unsafe { (self.lib.as_ref() as *const Library).as_ref().unwrap() }
    }

    fn lib_mut(&mut self) -> &'static mut Library<'static> {
        unsafe { (self.lib.as_mut() as *mut Library).as_mut().unwrap() }
    }
}

fn draw_layer(layer: &Layer, tex: &mut MappedTexture, dst_w: usize) {
    // RGBA order
    let mut color = layer.color.to_be_bytes();
    color[3] = 255 - color[3]; // Inverse alpha

    for y in 0..layer.height as usize {
        let dst_y = y + layer.y as usize;
        let dst_y_off = (dst_y * dst_w + layer.x as usize) * 4;
        let layer_y_off = y * layer.width as usize;

        let src_slice = &layer.bitmap[layer_y_off..layer_y_off + (layer.width as usize)];
        let dst_slice = &mut tex[dst_y_off..dst_y_off + (layer.width * 4) as usize];

        assert_eq!(dst_slice.len() % 4, 0);
        assert_eq!(src_slice.len() * 4, dst_slice.len());

        dst_slice
            .chunks_exact_mut(4)
            .zip(src_slice)
            .for_each(|(dst_chunk, k)| {
                let k = *k;

                let mut arr = u16x4::from_cast(u8x4::from_slice_unaligned(dst_chunk));
                arr *= (255 - k) as u16;

                let mut color_premul = u16x4::from_cast(u8x4::from_slice_unaligned(&color));
                color_premul *= k as u16;

                let result = u8x4::from_cast((arr + color_premul) / 255);
                result.write_to_slice_unaligned(dst_chunk);
            });
    }
}

fn clear_last(image: &[LastLayer], tex: &mut MappedTexture) {
    let tex_w = tex.width() as usize;
    for layer in image {
        for dst_y in layer.y..layer.y + layer.height {
            let dst_y_off = (dst_y * tex_w + layer.x as usize) * 4;
            let dst_slice = &mut tex[dst_y_off..dst_y_off + (layer.width * 4) as usize];
            dst_slice.fill(0);
        }
    }
}

#[derive(Debug)]
struct SubtitleParseError(subparse::errors::Error);

impl SubtitleParseError {
    pub fn new(inner: subparse::errors::Error) -> Self {
        Self(inner)
    }
}

impl Display for SubtitleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse subtitle: {}", self.0)
    }
}
impl Error for SubtitleParseError {}
