//! Palit "PALIT 150" RGB controller protocol (I2C dev 0x92, port 1).
//! Static-color sequence confirmed on hardware (RTX 3090, model 1502).

use crate::nvapi::NvApi;

pub const DEV: u8 = 0x92;
pub const PORT: u8 = 1;

const REG_COLORTBL: u8 = 0x6C;
const REG_PACKET: u8 = 0xE0;
const REG_COMMIT: u8 = 0x60;
const REG_FWID: u8 = 0xF0;

/// Set a solid static color on one GPU's LED.
/// Byte order on the wire is [R, G, B]; brightness 0..=100.
pub fn set_color(nv: &NvApi, gpu: usize, r: u8, g: u8, b: u8, br: u8) -> Result<(), String> {
    // two slots same color: avoids stale-slot blending
    nv.i2c_write(gpu, DEV, REG_COLORTBL, &[r, g, b, br, r, g, b, br], PORT)?;
    // header byte 0 = static effect
    nv.i2c_write(gpu, DEV, REG_PACKET, &[0, 0, 0, 0, r, g, b, br, 5], PORT)?;
    nv.i2c_write(gpu, DEV, REG_COMMIT, &[1], PORT)?;
    Ok(())
}

pub fn off(nv: &NvApi, gpu: usize) -> Result<(), String> {
    set_color(nv, gpu, 0, 0, 0, 0)
}

pub fn read_fw_id(nv: &NvApi, gpu: usize) -> Result<String, String> {
    let d = nv.i2c_read(gpu, DEV, REG_FWID, 16, PORT)?;
    let s: String = d
        .iter()
        .map(|&c| if (32..127).contains(&c) { c as char } else { '.' })
        .collect();
    Ok(s)
}

pub fn dump(nv: &NvApi, gpu: usize) -> Result<Vec<(u8, Vec<u8>)>, String> {
    let mut out = Vec::new();
    for reg in [REG_COMMIT, REG_COLORTBL, 0xB0, REG_PACKET, REG_FWID] {
        if let Ok(d) = nv.i2c_read(gpu, DEV, reg, 16, PORT) {
            out.push((reg, d));
        }
    }
    Ok(out)
}

/// HSV (h 0..360, s/v 0..1) -> RGB 0..255. Integer-friendly, no float libs needed.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

const GREEN: (u8, u8, u8) = (0, 255, 0);
const YELLOW: (u8, u8, u8) = (255, 255, 0);
const ORANGE: (u8, u8, u8) = (255, 140, 0);
const RED: (u8, u8, u8) = (255, 0, 0);

fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), f: f32) -> (u8, u8, u8) {
    let f = f.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) as u8;
    (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// Temperature -> color over 4 zones: green / yellow / orange / red.
/// Discrete zones by default; `smooth` interpolates between zone colors.
///   green  : t <= green_max
///   yellow : green_max < t <= yellow_max
///   orange : yellow_max < t <= orange_max
///   red    : t > orange_max  (fully red at/after `red_full` when smooth)
pub fn temp_to_color(
    t: i32,
    green_max: i32,
    yellow_max: i32,
    orange_max: i32,
    red_full: i32,
    smooth: bool,
) -> (u8, u8, u8) {
    if !smooth {
        return if t <= green_max {
            GREEN
        } else if t <= yellow_max {
            YELLOW
        } else if t <= orange_max {
            ORANGE
        } else {
            RED
        };
    }
    let span = |lo: i32, hi: i32| (t - lo) as f32 / (hi - lo).max(1) as f32;
    if t <= green_max {
        GREEN
    } else if t <= yellow_max {
        lerp(GREEN, YELLOW, span(green_max, yellow_max))
    } else if t <= orange_max {
        lerp(YELLOW, ORANGE, span(yellow_max, orange_max))
    } else if t < red_full {
        lerp(ORANGE, RED, span(orange_max, red_full))
    } else {
        RED
    }
}
