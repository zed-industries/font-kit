// font-kit/src/loaders/directwrite.rs
//
// Copyright © 2018 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use dwrote::FontFace as DWriteFontFace;
use dwrote::FontFile as DWriteFontFile;
use dwrote::FontStyle as DWriteFontStyle;
use dwrote::GlyphOffset as DWriteGlyphOffset;
use dwrote::GlyphRunAnalysis as DWriteGlyphRunAnalysis;
use dwrote::InformationalStringId as DWriteInformationalStringId;
use dwrote::{DWRITE_GLYPH_RUN, DWRITE_MEASURING_MODE_NATURAL, DWRITE_RENDERING_MODE_ALIASED};
use dwrote::{DWRITE_RENDERING_MODE_NATURAL, DWRITE_TEXTURE_ALIASED_1x1};
use dwrote::{DWRITE_TEXTURE_CLEARTYPE_3x1, OutlineBuilder};
use euclid::{Point2D, Rect, Size2D, Vector2D};
use lyon_path::PathEvent;
use lyon_path::builder::PathBuilder;
use std::fmt::{self, Debug, Formatter};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Deref;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use winapi::shared::minwindef::FALSE;

use canvas::{Canvas, Format, RasterizationOptions};
use descriptor::{FONT_STRETCH_MAPPING, Properties, Stretch, Style, Weight};
use font::{Face, HintingOptions, Metrics, Type};

pub type NativeFont = DWriteFontFace;

pub struct Font {
    dwrite_font_face: DWriteFontFace,
    cached_data: Mutex<Option<Arc<Vec<u8>>>>,
}

impl Font {
    pub fn from_bytes(font_data: Arc<Vec<u8>>, font_index: u32) -> Result<Font, ()> {
        let font_file = try!(DWriteFontFile::new_from_data(&**font_data).ok_or(()));
        let face = font_file.create_face(font_index, 0);
        Ok(Font {
            dwrite_font_face: face,
            cached_data: Mutex::new(Some(font_data)),
        })
    }

    pub fn from_file(file: &mut File, font_index: u32) -> Result<Font, ()> {
        let mut font_data = vec![];
        try!(file.seek(SeekFrom::Start(0)).map_err(drop));
        try!(file.read_to_end(&mut font_data).map_err(drop));
        Font::from_bytes(Arc::new(font_data), font_index)
    }

    #[inline]
    pub fn from_path<P>(path: P, font_index: u32) -> Result<Font, ()> where P: AsRef<Path> {
        <Font as Face>::from_path(path, font_index)
    }

    // TODO(pcwalton)
    pub unsafe fn from_native_font(dwrite_font_face: NativeFont) -> Font {
        Font {
            dwrite_font_face,
            cached_data: Mutex::new(None),
        }
    }

    pub fn analyze_bytes(font_data: Arc<Vec<u8>>) -> Result<Type, ()> {
        match DWriteFontFile::analyze_data(&**font_data) {
            0 => Err(()),
            1 => Ok(Type::Single),
            font_count => Ok(Type::Collection(font_count)),
        }
    }

    pub fn analyze_file(file: &mut File) -> Result<Type, ()> {
        let mut font_data = vec![];
        try!(file.seek(SeekFrom::Start(0)).map_err(drop));
        match file.read_to_end(&mut font_data) {
            Err(_) => Err(()),
            Ok(_) => Font::analyze_bytes(Arc::new(font_data)),
        }
    }

    #[inline]
    pub fn analyze_path<P>(path: P) -> Result<Type, ()> where P: AsRef<Path> {
        <Self as Face>::analyze_path(path)
    }

    #[inline]
    pub fn postscript_name(&self) -> String {
        let dwrite_font = self.dwrite_font_face.get_font();
        dwrite_font.informational_string(DWriteInformationalStringId::PostscriptName)
                   .unwrap_or_else(|| dwrite_font.family_name())
    }

    #[inline]
    pub fn full_name(&self) -> String {
        let dwrite_font = self.dwrite_font_face.get_font();
        dwrite_font.informational_string(DWriteInformationalStringId::FullName)
                   .unwrap_or_else(|| dwrite_font.family_name())
    }

    #[inline]
    pub fn family_name(&self) -> String {
        self.dwrite_font_face.get_font().family_name()
    }

    // FIXME(pcwalton)
    #[inline]
    pub fn is_monospace(&self) -> bool {
        true
    }

    pub fn properties(&self) -> Properties {
        let dwrite_font = self.dwrite_font_face.get_font();
        Properties {
            style: style_for_dwrite_style(dwrite_font.style()),
            stretch: Stretch(FONT_STRETCH_MAPPING[(dwrite_font.stretch() as usize) - 1]),
            weight: Weight(dwrite_font.weight() as u32 as f32),
        }
    }

    pub fn glyph_for_char(&self, character: char) -> Option<u32> {
        let chars = [character as u32];
        self.dwrite_font_face.get_glyph_indices(&chars).into_iter().next().map(|g| g as u32)
    }

    pub fn outline<B>(&self, glyph_id: u32, _: HintingOptions, path_builder: &mut B)
                      -> Result<(), ()>
                      where B: PathBuilder {
        let outline_buffer = OutlineBuffer::new();
        self.dwrite_font_face.get_glyph_run_outline(self.metrics().units_per_em as f32,
                                                    &[glyph_id as u16],
                                                    None,
                                                    None,
                                                    false,
                                                    false,
                                                    Box::new(outline_buffer.clone()));
        outline_buffer.flush(path_builder);
        Ok(())
    }

    pub fn typographic_bounds(&self, glyph_id: u32) -> Rect<f32> {
        let metrics = self.dwrite_font_face.get_design_glyph_metrics(&[glyph_id as u16], false);

        let metrics = &metrics[0];
        let advance_width = metrics.advanceWidth as i32;
        let advance_height = metrics.advanceHeight as i32;
        let left_side_bearing = metrics.leftSideBearing as i32;
        let right_side_bearing = metrics.rightSideBearing as i32;
        let top_side_bearing = metrics.topSideBearing as i32;
        let bottom_side_bearing = metrics.bottomSideBearing as i32;
        let vertical_origin_y = metrics.verticalOriginY as i32;

        let y_offset = vertical_origin_y + bottom_side_bearing - advance_height;
        let width = advance_width - (left_side_bearing + right_side_bearing);
        let height = advance_height - (top_side_bearing + bottom_side_bearing);

        Rect::new(Point2D::new(left_side_bearing as f32, y_offset as f32),
                  Size2D::new(width as f32, height as f32))
    }

    pub fn advance(&self, glyph_id: u32) -> Vector2D<f32> {
        let metrics = self.dwrite_font_face.get_design_glyph_metrics(&[glyph_id as u16], false);
        let metrics = &metrics[0];
        Vector2D::new(metrics.advanceWidth as f32, 0.0)
    }

    pub fn origin(&self, _: u32) -> Point2D<f32> {
        // FIXME(pcwalton): This can't be right!
        Point2D::zero()
    }

    pub fn metrics(&self) -> Metrics {
        let dwrite_font = self.dwrite_font_face.get_font();
        let dwrite_metrics = dwrite_font.metrics();
        Metrics {
            units_per_em: dwrite_metrics.designUnitsPerEm as u32,
            ascent: dwrite_metrics.ascent as f32,
            descent: -(dwrite_metrics.descent as f32),
            line_gap: dwrite_metrics.lineGap as f32,
            cap_height: dwrite_metrics.capHeight as f32,
            x_height: dwrite_metrics.xHeight as f32,
            underline_position: dwrite_metrics.underlinePosition as f32,
            underline_thickness: dwrite_metrics.underlineThickness as f32,
        }
    }

    pub fn font_data(&self) -> Option<FontData> {
        let mut font_data = self.cached_data.lock().unwrap();
        if font_data.is_none() {
            let files = self.dwrite_font_face.get_files();
            // FIXME(pcwalton): Is this right? When can a font have multiple files?
            if let Some(file) = files.get(0) {
                *font_data = Some(Arc::new(file.get_font_file_bytes()))
            }
        }

        if font_data.is_none() {
            None
        } else {
            Some(FontData {
                font_data,
            })
        }
    }

    #[inline]
    pub fn raster_bounds(&self,
                         glyph_id: u32,
                         point_size: f32,
                         origin: &Point2D<f32>,
                         hinting_options: HintingOptions,
                         rasterization_options: RasterizationOptions)
                         -> Rect<i32> {
        <Self as Face>::raster_bounds(self,
                                      glyph_id,
                                      point_size,
                                      origin,
                                      hinting_options,
                                      rasterization_options)
    }

    // TODO(pcwalton): This is woefully incomplete. See WebRender's code for a more complete
    // implementation.
    pub fn rasterize_glyph(&self,
                           canvas: &mut Canvas,
                           glyph_id: u32,
                           point_size: f32,
                           origin: &Point2D<f32>,
                           hinting_options: HintingOptions,
                           rasterization_options: RasterizationOptions) {
        let dwrite_analysis = self.build_glyph_analysis(glyph_id,
                                                        point_size,
                                                        origin,
                                                        hinting_options,
                                                        rasterization_options);

        let texture_type = match rasterization_options {
            RasterizationOptions::Bilevel => DWRITE_TEXTURE_ALIASED_1x1,
            RasterizationOptions::GrayscaleAa | RasterizationOptions::SubpixelAa => {
                DWRITE_TEXTURE_CLEARTYPE_3x1
            }
        };

        // TODO(pcwalton): Avoid a copy in some cases by writing directly to the canvas.
        let texture_bounds = dwrite_analysis.get_alpha_texture_bounds(texture_type);
        let texture_format = if texture_type == DWRITE_TEXTURE_ALIASED_1x1 {
            Format::A8
        } else {
            Format::Rgb24
        };
        let texture_bits_per_pixel = texture_format.bits_per_pixel();
        let texture_bytes_per_pixel = texture_bits_per_pixel as usize / 8;
        let texture_width = texture_bounds.right - texture_bounds.left;
        let texture_height = texture_bounds.bottom - texture_bounds.top;
        let texture_size = Size2D::new(texture_width, texture_height).to_u32();
        let texture_stride = texture_width as usize * texture_bytes_per_pixel;

        let mut texture_bytes = dwrite_analysis.create_alpha_texture(texture_type, texture_bounds);
        canvas.blit_from(&mut texture_bytes,
                         &texture_size,
                         texture_stride,
                         texture_format);
    }

    fn build_glyph_analysis(&self,
                            glyph_id: u32,
                            point_size: f32,
                            origin: &Point2D<f32>,
                            hinting_options: HintingOptions,
                            rasterization_options: RasterizationOptions)
                            -> DWriteGlyphRunAnalysis {
        unsafe {
            let glyph_id = glyph_id as u16;
            let advance = 0.0;
            let offset = DWriteGlyphOffset {
                advanceOffset: 0.0,
                ascenderOffset: 0.0,
            };
            let glyph_run = DWRITE_GLYPH_RUN {
                fontFace: self.dwrite_font_face.as_ptr(),
                fontEmSize: point_size,
                glyphCount: 1,
                glyphIndices: &glyph_id,
                glyphAdvances: &advance,
                glyphOffsets: &offset,
                isSideways: FALSE,
                bidiLevel: 0,
            };

            let rendering_mode = match rasterization_options {
                RasterizationOptions::Bilevel => DWRITE_RENDERING_MODE_ALIASED,
                RasterizationOptions::GrayscaleAa | RasterizationOptions::SubpixelAa => {
                    DWRITE_RENDERING_MODE_NATURAL
                }
            };

            DWriteGlyphRunAnalysis::create(&glyph_run,
                                           1.0,
                                           None,
                                           rendering_mode,
                                           DWRITE_MEASURING_MODE_NATURAL,
                                           0.0,
                                           0.0)
        }
    }
}

impl Clone for Font {
    #[inline]
    fn clone(&self) -> Font {
        Font {
            dwrite_font_face: self.dwrite_font_face.clone(),
            cached_data: Mutex::new((*self.cached_data.lock().unwrap()).clone())
        }
    }
}

impl Debug for Font {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), fmt::Error> {
        self.family_name().fmt(fmt)
    }
}

impl Face for Font {
    type NativeFont = NativeFont;

    #[inline]
    fn from_bytes(font_data: Arc<Vec<u8>>, font_index: u32) -> Result<Self, ()> {
        Font::from_bytes(font_data, font_index)
    }

    #[inline]
    fn from_file(file: &mut File, font_index: u32) -> Result<Font, ()> {
        Font::from_file(file, font_index)
    }

    #[inline]
    unsafe fn from_native_font(native_font: Self::NativeFont) -> Self {
        Font::from_native_font(native_font)
    }

    #[inline]
    fn analyze_file(file: &mut File) -> Result<Type, ()> {
        Font::analyze_file(file)
    }

    #[inline]
    fn postscript_name(&self) -> String {
        self.postscript_name()
    }

    #[inline]
    fn full_name(&self) -> String {
        self.full_name()
    }

    #[inline]
    fn family_name(&self) -> String {
        self.family_name()
    }

    #[inline]
    fn is_monospace(&self) -> bool {
        self.is_monospace()
    }

    #[inline]
    fn properties(&self) -> Properties {
        self.properties()
    }

    #[inline]
    fn glyph_for_char(&self, character: char) -> Option<u32> {
        self.glyph_for_char(character)
    }

    #[inline]
    fn outline<B>(&self, glyph_id: u32, hinting: HintingOptions, path_builder: &mut B)
                  -> Result<(), ()>
                  where B: PathBuilder {
        self.outline(glyph_id, hinting, path_builder)
    }

    #[inline]
    fn typographic_bounds(&self, glyph_id: u32) -> Rect<f32> {
        self.typographic_bounds(glyph_id)
    }

    #[inline]
    fn advance(&self, glyph_id: u32) -> Vector2D<f32> {
        self.advance(glyph_id)
    }

    #[inline]
    fn origin(&self, origin: u32) -> Point2D<f32> {
        self.origin(origin)
    }

    #[inline]
    fn metrics(&self) -> Metrics {
        self.metrics()
    }

    #[inline]
    fn rasterize_glyph(&self,
                       canvas: &mut Canvas,
                       glyph_id: u32,
                       point_size: f32,
                       origin: &Point2D<f32>,
                       hinting_options: HintingOptions,
                       rasterization_options: RasterizationOptions) {
        self.rasterize_glyph(canvas,
                             glyph_id,
                             point_size,
                             origin,
                             hinting_options,
                             rasterization_options)
    }
}

pub struct FontData<'a> {
    font_data: MutexGuard<'a, Option<Arc<Vec<u8>>>>,
}

impl<'a> Deref for FontData<'a> {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        &***self.font_data.as_ref().unwrap()
    }
}

#[derive(Clone)]
struct OutlineBuffer {
    path_events: Arc<Mutex<Vec<PathEvent>>>,
}

impl OutlineBuffer {
    pub fn new() -> OutlineBuffer {
        OutlineBuffer {
            path_events: Arc::new(Mutex::new(vec![])),
        }
    }

    pub fn flush<PB>(&self, path_builder: &mut PB) where PB: PathBuilder {
        let mut path_events = self.path_events.lock().unwrap();
        for path_event in path_events.drain(..) {
            path_builder.path_event(path_event)
        }
    }
}

impl OutlineBuilder for OutlineBuffer {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path_events.lock().unwrap().push(PathEvent::MoveTo(Point2D::new(x, -y)))
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path_events.lock().unwrap().push(PathEvent::LineTo(Point2D::new(x, -y)))

    }
    fn curve_to(&mut self, cp0x: f32, cp0y: f32, cp1x: f32, cp1y: f32, x: f32, y: f32) {
        self.path_events.lock().unwrap().push(PathEvent::CubicTo(Point2D::new(cp0x, -cp0y),
                                                                 Point2D::new(cp1x, -cp1y),
                                                                 Point2D::new(x, -y)))

    }
    fn close(&mut self) {
        self.path_events.lock().unwrap().push(PathEvent::Close)
    }
}

fn style_for_dwrite_style(style: DWriteFontStyle) -> Style {
    match style {
        DWriteFontStyle::Normal => Style::Normal,
        DWriteFontStyle::Oblique => Style::Oblique,
        DWriteFontStyle::Italic => Style::Italic,
    }
}