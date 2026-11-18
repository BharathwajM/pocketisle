//! Pocket Isle: a native dynamic island for Windows that blends into a YASB bar.
//!
//! `pocketisle.exe`              runs the island (one instance per user session).
//! `pocketisle.exe hook claude`  forwards a Claude Code hook event (JSON on stdin) to the island.
//! `pocketisle.exe hook codex J` forwards a Codex `notify` event (JSON as the last argument).
//! `pocketisle.exe play G [T]`   plays a DS / GBA / Game Boy game in a window (the Game tab).
//! `pocketisle.exe demo`         the island with made-up data, touring its features by itself.

#![windows_subsystem = "windows"]

mod agent;
mod ask;
mod anim;
mod app;
mod audio;
mod brightness;
mod calendar;
mod config;
mod devices;
mod dex;
mod emu;
mod games;
mod github;
mod heat;
mod controls;
mod demo;
mod events;
mod imaging;
mod media;
mod national;
mod notes;
mod net;
mod notify;
mod otp;
mod peeks;
mod pokemon;
mod privacy;
mod procs;
mod quest;
mod shelf;
mod sound;
mod render;
mod scan;
mod sysinfo;
mod theme;
mod usage;
mod util;
mod wallpaper;
mod weather;
mod wpe;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("ask") {
        ask::run();
        return;
    }
    if args.get(1).map(String::as_str) == Some("play") {
        emu::run(&args[2..]);
        return;
    }
    if args.get(1).map(String::as_str) == Some("hook") {
        agent::run_hook_cli(&args[2..]);
        return;
    }
    if args.get(1).map(String::as_str) == Some("demo") {
        util::set_demo();
        demo::prepare();
    }
    if let Err(e) = app::run() {
        util::log(&format!("fatal: {e:?}"));
    }
}
