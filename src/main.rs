use crate::config::{ConfigFile, Modifier, Operation, RatchetMode, Action};
use crate::hid::HidHandler;
use crate::x11::X11Handler;
use std::process::Command;

mod x11;
mod hid;
mod config;
mod udev;

pub(crate) mod keysyms {
    include!(concat!(env!("OUT_DIR"), "/keysyms.rs"));
}

#[derive(Debug)]
pub(crate) enum StateChanges {
    FocusChanged { pid: u32, program: String },
    ModifiersChanged { modifiers: u8 },
    CrownTouched { modifiers: u8 },
    CrownReleased { modifiers: u8 },
    CrownClicked { modifiers: u8 },
    CrownRotated { modifiers: u8, amount: i16, notch_amount: i16, pressed: bool },
}

fn execute_commands(commands: &[Operation], x11_handler: &X11Handler, debug_enabled: bool) {
    for command in commands {
        if debug_enabled {
            println!("Exec {:?}", command);
        }
        match command {
            Operation::KeyPress(keysym, modifiers) => {
                x11_handler.send_key(*keysym, *modifiers);
            }
            Operation::Execute(command) => {
                let mut parts = command.split_ascii_whitespace();
                if let Some(cmd) = parts.next() {
                    let _ = Command::new(cmd).args(parts).spawn();
                }
            }
        }
    }
}

fn main() -> () {
    let mut args = pico_args::Arguments::from_env();

    let debug_enabled: bool = args.contains(["-d", "--debug"]);

    let (sender, receiver) = crossbeam_channel::unbounded();
    let x11_handler = X11Handler::new(sender.clone(), debug_enabled).unwrap();
    let hid_handler = HidHandler::new(sender.clone(), debug_enabled).unwrap();
    let mut config = ConfigFile::new();
    let mut last_mode = RatchetMode::Ratcheted;
    let mut last_modifiers = Modifier::None;

    loop {
        let res = receiver.recv().unwrap();
        if debug_enabled {
            println!("Processing {:?}", res);
        }
        match res {
            StateChanges::FocusChanged { program, .. } => {
                config.select_app(&program);
                let mode = config.ratchet_mode_for_modifier(last_modifiers);
                if mode != last_mode {
                    last_mode = mode;
                    match mode {
                        RatchetMode::Ratcheted => hid_handler.enable_ratcher(),
                        _ => hid_handler.disable_ratcher(),
                    };
                }
            }
            StateChanges::ModifiersChanged { modifiers } => {
                let modifiers = Modifier::from(modifiers);
                if last_modifiers != modifiers {
                    last_modifiers = modifiers;
                    let mode = config.ratchet_mode_for_modifier(modifiers);
                    if mode != last_mode {
                        last_mode = mode;
                        match mode {
                            RatchetMode::Ratcheted => hid_handler.enable_ratcher(),
                            _ => hid_handler.disable_ratcher(),
                        };
                    }
                }
            }
            StateChanges::CrownRotated { modifiers, amount, pressed, notch_amount, .. } => {
                let modifiers = Modifier::from(modifiers);
                let action = match (amount, pressed) {
                    (amount, true) if amount > 0 => Action::RightPressed,
                    (amount, true) if amount < 0 => Action::LeftPressed,
                    (amount, _) if amount > 0 => Action::Right,
                    (amount, _) if amount < 0 => Action::Left,
                    _ => continue
                };
                if last_mode == RatchetMode::Ratcheted && notch_amount == 0 {
                    continue;
                }
                if let Some(actions) = config.get_actions_for_modifiers(modifiers, action) {
                    execute_commands(actions, &x11_handler, debug_enabled);
                }
            }
            StateChanges::CrownTouched {modifiers} => {
                let modifiers = Modifier::from(modifiers);
                if let Some(actions) = config.get_actions_for_modifiers(modifiers, Action::Touch) {
                    execute_commands(actions, &x11_handler, debug_enabled);
                }
            }
            StateChanges::CrownReleased {modifiers} => {
                let modifiers = Modifier::from(modifiers);
                if let Some(actions) = config.get_actions_for_modifiers(modifiers, Action::Release) {
                    execute_commands(actions, &x11_handler, debug_enabled);
                }
            }
            StateChanges::CrownClicked { modifiers } => {
                let modifiers = Modifier::from(modifiers);
                if let Some(actions) = config.get_actions_for_modifiers(modifiers, Action::Click) {
                    execute_commands(actions, &x11_handler, debug_enabled);
                }
            }
        }
    }
}
