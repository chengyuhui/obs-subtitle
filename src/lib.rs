#![feature(slice_fill)]
use std::{borrow::Cow, cell::RefCell, sync::RwLock};

use obs_wrapper::graphics::GraphicsColorFormat;
use obs_wrapper::graphics::GraphicsTexture;
use obs_wrapper::{log::Logger, obs_register_module, obs_string, prelude::*, source::*};
mod ass;

use ass::AssData;
use log::*;

macro_rules! ensure_data {
    ($i:ident) => {
        if let Some(data) = $i {
            data
        } else {
            return;
        }
    };
}

struct SubtitleModule {
    context: ModuleContext,
}

struct SourceData {
    _src: SourceContext,
    ass: AssData,
    tex: GraphicsTexture,
    state: MediaState,

    canvas_w: u32,
    canvas_h: u32,

    playlist: RwLock<Vec<String>>,
}

impl SourceData {
    fn load_track(&mut self, path: &str) {
        // "/home/harryc/workspace/obs-subtitle/subs/test1.ass"
        if let Err(e) = self.ass.load_track(path) {
            error!("Failed to load track: {}", e);
        }
    }
}

impl Sourceable for SubtitleModule {
    fn get_id() -> ObsString {
        obs_string!("subtitle_source")
    }

    fn get_type() -> SourceType {
        SourceType::INPUT
    }
}

impl GetNameSource<SourceData> for SubtitleModule {
    fn get_name() -> ObsString {
        obs_string!("Subtitle Input")
    }
}

impl CreatableSource<SourceData> for SubtitleModule {
    fn create(
        context: &mut CreatableSourceContext<SourceData>,
        mut source: SourceContext,
    ) -> SourceData {
        let width = 1920;
        let height = 1080;

        let tex = GraphicsTexture::new(width as u32, height as u32, GraphicsColorFormat::RGBA);
        let ass = AssData::new().unwrap();

        context.register_hotkey(
            obs_string!("Preheat.PlayPause"),
            obs_string!("Play/Pause"),
            |key, data| {
                if key.pressed {
                    let data = ensure_data!(data);
                    if data.state == MediaState::Playing {}
                }
            },
        );

        source.update_source_settings(&context.settings);

        let data = SourceData {
            _src: source,
            tex,

            ass,
            state: MediaState::Playing,

            canvas_h: height,
            canvas_w: width,

            playlist: Default::default(),
        };

        data
    }
}

impl VideoRenderSource<SourceData> for SubtitleModule {
    fn video_render(
        data: &mut Option<SourceData>,
        _context: &mut GlobalContext,
        _render: &mut VideoRenderContext,
    ) {
        let data = ensure_data!(data);
        data.tex.draw(0, 0, 0, 0, false);
    }
}

impl GetPropertiesSource<SourceData> for SubtitleModule {
    fn get_properties(_data: &mut Option<SourceData>, properties: &mut Properties) {
        properties
            .add_int(
                obs_string!("canvas_height"),
                obs_string!("Canvas Height"),
                800,
                3840 * 3,
                1,
                false,
            )
            .add_int(
                obs_string!("canvas_width"),
                obs_string!("Canvas Width"),
                600,
                3840 * 3,
                1,
                false,
            )
            .add_editable_list(
                obs_string!("playlist"),
                obs_string!("Playlist"),
                EditableListType::Files,
                obs_string!("ASS subtitle file (*.ass)"),
                obs_string!(""),
            );
    }
}

impl UpdateSource<SourceData> for SubtitleModule {
    fn update(data: &mut Option<SourceData>, settings: &mut DataObj, _context: &mut GlobalContext) {
        let data = ensure_data!(data);

        let mut playlist = data.playlist.write().unwrap();
        playlist.clear();

        let new_list: DataArray = settings.get(obs_string!("playlist")).unwrap();

        for i in 0..new_list.len() {
            let item = new_list.get(i).unwrap();
            let path: Cow<str> = item.get(obs_string!("value")).unwrap();
            info!("New playlist path: {}", path);
            playlist.push(path.into_owned());
        }

        println!("{} {}", data.ass.loaded(), playlist.is_empty());

        if !data.ass.loaded() && !playlist.is_empty() {
            let path = playlist[0].clone();
            drop(playlist);
            data.load_track(&path);
        }
    }
}

impl GetHeightSource<SourceData> for SubtitleModule {
    fn get_height(data: &mut Option<SourceData>) -> u32 {
        data.as_ref().map(|data| data.canvas_h).unwrap_or(0)
    }
}

impl GetWidthSource<SourceData> for SubtitleModule {
    fn get_width(data: &mut Option<SourceData>) -> u32 {
        data.as_ref().map(|data| data.canvas_w).unwrap_or(0)
    }
}

impl MediaPlayPauseSource<SourceData> for SubtitleModule {
    fn play_pause(data: &mut Option<SourceData>, pause: bool) {
        let data = ensure_data!(data);
        match (&data.state, pause) {
            (MediaState::Playing, true) => data.state = MediaState::Paused,
            (MediaState::Paused, false) => data.state = MediaState::Playing,
            _ => {}
        }
    }
}

impl MediaGetStateSource<SourceData> for SubtitleModule {
    fn get_state(data: &mut Option<SourceData>) -> MediaState {
        data.as_ref()
            .map(|data| data.state)
            .unwrap_or(MediaState::None)
    }
}

impl MediaGetTimeSource<SourceData> for SubtitleModule {
    fn get_time(data: &mut Option<SourceData>) -> i64 {
        data.as_ref()
            .map(|data| data.ass.current_time())
            .unwrap_or(0)
    }
}

impl MediaGetDurationSource<SourceData> for SubtitleModule {
    fn get_duration(data: &mut Option<SourceData>) -> i64 {
        data.as_ref()
            .map(|data| data.ass.current_len())
            .unwrap_or(0)
    }
}

impl VideoTickSource<SourceData> for SubtitleModule {
    fn video_tick(data: &mut Option<SourceData>, seconds: f32) {
        let data = ensure_data!(data);
        if data.state == MediaState::Playing {
            data.ass.tick((seconds * 1000.0) as i64, &mut data.tex);
        }
    }
}

impl Module for SubtitleModule {
    fn new(context: ModuleContext) -> Self {
        let _ = Logger::new().init();
        SubtitleModule { context }
    }

    fn get_ctx(&self) -> &ModuleContext {
        &self.context
    }

    fn load(&mut self, load_context: &mut LoadContext) -> bool {
        let source = load_context
            .create_source_builder::<SubtitleModule, SourceData>()
            .enable_get_name()
            .enable_create()
            .enable_update()
            .enable_get_properties()
            .enable_get_width()
            .enable_get_height()
            .enable_video_render()
            .enable_video_tick()
            .enable_media_get_state()
            .enable_media_play_pause()
            .enable_media_get_time()
            .enable_media_get_duration()
            .build();

        load_context.register_source(source);

        true
    }

    fn description() -> ObsString {
        obs_string!("A module that renders ASS subtitle onto your scene.")
    }

    fn name() -> ObsString {
        obs_string!("Subtitle Module")
    }

    fn author() -> ObsString {
        obs_string!("Harry Cheng")
    }
}

obs_register_module!(SubtitleModule);
