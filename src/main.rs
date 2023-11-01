#![allow(non_snake_case)]

// use dioxus_desktop::{Config, WindowBuilder};

use iced::{Application, Settings, window, Font, font};

mod search;
mod ui;

fn main() -> iced::Result {
    ui::Keal::run(Settings {
        window: window::Settings {
            size: (1920/3, 1080/2),
            position: window::Position::Centered,
            resizable: false,
            decorations: false,
            transparent: true,
            level: window::Level::AlwaysOnTop,
            ..Default::default()
        },
        default_font: Font {
            family: font::Family::Name("Iosevka"),
            weight: font::Weight::Normal,
            stretch: font::Stretch::Normal,
            monospaced: false
        },
        ..Default::default()
    })
}
