//! UI Theme colors
//!
//! Defines the color palette used throughout the UI system.

use embedded_graphics::pixelcolor::Rgb888;

pub const BACKGROUND: Rgb888 = Rgb888::new(24, 24, 32);
pub const FOREGROUND: Rgb888 = Rgb888::new(220, 220, 230);
pub const ACCENT: Rgb888 = Rgb888::new(80, 140, 200);
pub const ACCENT_HIGHLIGHT: Rgb888 = Rgb888::new(100, 160, 220);
pub const SUCCESS: Rgb888 = Rgb888::new(80, 200, 120);
pub const WARNING: Rgb888 = Rgb888::new(230, 180, 80);
pub const ERROR: Rgb888 = Rgb888::new(220, 80, 80);
pub const BORDER: Rgb888 = Rgb888::new(60, 60, 80);
pub const BUTTON_BG: Rgb888 = Rgb888::new(50, 50, 70);
pub const BUTTON_SELECTED: Rgb888 = Rgb888::new(80, 140, 200);
