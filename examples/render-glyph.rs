// font-kit/examples/render-glyph.rs
//
// Copyright © 2018 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

extern crate clap;
extern crate colored;
extern crate euclid;
extern crate font_kit;

use clap::{App, Arg, ArgGroup, ArgMatches};
use colored::Colorize;
use euclid::Point2D;
use font_kit::canvas::{Canvas, Format, RasterizationOptions};
use font_kit::hinting::HintingOptions;
use font_kit::source::SystemSource;
use std::fmt::Write;

fn get_args() -> ArgMatches<'static> {
    let postscript_name_arg =
        Arg::with_name("POSTSCRIPT-NAME").help("PostScript name of the font")
                                         .default_value("ArialMT")
                                         .index(1);
    let glyph_arg = Arg::with_name("GLYPH").help("Character to render")
                                           .default_value("A")
                                           .index(2);
    let size_arg = Arg::with_name("SIZE").help("Font size in blocks")
                                         .default_value("32")
                                         .index(3);
    let grayscale_arg = Arg::with_name("grayscale").long("grayscale")
                                                   .help("Use grayscale antialiasing (default)");
    let bilevel_arg = Arg::with_name("bilevel").help("Use bilevel (black & white) rasterization")
                                               .short("b")
                                               .long("bilevel");
    let subpixel_arg = Arg::with_name("subpixel").help("Use subpixel (LCD) rasterization")
                                                 .short("s")
                                                 .long("subpixel");
    let hinting_arg = Arg::with_name("hinting").help("Select hinting type")
                                               .short("H")
                                               .long("hinting")
                                               .takes_value(true)
                                               .possible_value("vertical")
                                               .possible_value("full")
                                               .value_names(&["TYPE"]);
    let rasterization_mode_group =
        ArgGroup::with_name("rasterization-mode").args(&["grayscale", "bilevel", "subpixel"]);
    App::new("render-glyph").version("0.1")
                            .author("The Pathfinder Project Developers")
                            .about("Simple example tool to render glyphs with `font-kit`")
                            .arg(postscript_name_arg)
                            .arg(glyph_arg)
                            .arg(size_arg)
                            .arg(grayscale_arg)
                            .arg(bilevel_arg)
                            .arg(subpixel_arg)
                            .group(rasterization_mode_group)
                            .arg(hinting_arg)
                            .get_matches()
}

fn main() {
    let matches = get_args();

    let postscript_name = matches.value_of("POSTSCRIPT-NAME").unwrap();
    let character = matches.value_of("GLYPH").unwrap().as_bytes()[0] as char;
    let size: f32 = matches.value_of("SIZE").unwrap().parse().unwrap();

    let (canvas_format, rasterization_options) = if matches.is_present("bilevel") {
        (Format::A8, RasterizationOptions::Bilevel)
    } else if matches.is_present("subpixel") {
        (Format::Rgb24, RasterizationOptions::SubpixelAa)
    } else {
        (Format::A8, RasterizationOptions::GrayscaleAa)
    };

    let hinting_options = match matches.value_of("hinting") {
        Some(value) if value == "vertical" => HintingOptions::Vertical(size),
        Some(value) if value == "full" => HintingOptions::Full(size),
        _ => HintingOptions::None,
    };

    let font = SystemSource::new().select_by_postscript_name(&postscript_name)
                                  .unwrap()
                                  .load()
                                  .unwrap();
    let glyph_id = font.glyph_for_char(character).unwrap();

    let raster_rect = font.raster_bounds(glyph_id,
                                         size,
                                         &Point2D::zero(),
                                         hinting_options,
                                         rasterization_options)
                          .unwrap();

    let mut canvas = Canvas::new(&raster_rect.size.to_u32(), canvas_format);

    let origin = Point2D::new(-raster_rect.origin.x, -raster_rect.origin.y).to_f32();
    font.rasterize_glyph(&mut canvas,
                         glyph_id,
                         size,
                         &origin,
                         hinting_options,
                         rasterization_options)
        .unwrap();

    println!("glyph {}:", glyph_id);
    for y in 0..raster_rect.size.height {
        let mut line = String::new();
        let (row_start, row_end) = (y as usize * canvas.stride, (y + 1) as usize * canvas.stride);
        let row = &canvas.pixels[row_start..row_end];
        for x in 0..raster_rect.size.width {
            match canvas.format {
                Format::Rgba32 => unimplemented!(),
                Format::Rgb24 => {
                    write!(&mut line,
                           "{}{}{}",
                           shade(row[x as usize * 3 + 0]).to_string().red(),
                           shade(row[x as usize * 3 + 1]).to_string().green(),
                           shade(row[x as usize * 3 + 2]).to_string().blue()).unwrap();
                }
                Format::A8 => {
                    let shade = shade(row[x as usize]);
                    line.push(shade);
                    line.push(shade);
                }
            }
        }
        println!("{}", line);
    }
}

fn shade(value: u8) -> char {
    match value {
        0 => ' ',
        1...84 => '░',
        85...169 => '▒',
        170...254 => '▓',
        _ => '█',
    }
}
