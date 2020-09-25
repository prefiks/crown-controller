use crate::config::{ConfigFile, Modifier, Operation, RatchetMode};
use crate::hid::HidHandler;
use crate::x11::X11Handler;

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
            _ => {}
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

    loop {
        let res = receiver.recv().unwrap();
        match res {
            StateChanges::FocusChanged { program, .. } => {
                config.select_app(&program);
                match config.get_mapping_for_modifiers(Modifier::None) {
                    (RatchetMode::Ratcheted, _) => hid_handler.enable_ratcher(),
                    _ => hid_handler.disable_ratcher(),
                };
            }
            StateChanges::CrownRotated { modifiers, amount, pressed, notch_amount, .. } => {
                let modifiers = Modifier::from(modifiers);
                if let (mode, Some(commands)) = config.get_mapping_for_modifiers(modifiers) {
                    let commands = match (mode, notch_amount, amount, pressed) {
                        (RatchetMode::Ratcheted, na, ..) if na == 0 => continue,
                        (_, _, amount, true) if amount > 0 => &commands.right_pressed,
                        (_, _, amount, true) if amount < 0 => &commands.left_pressed,
                        (_, _, amount, _) if amount > 0 => &commands.right,
                        (_, _, amount, _) if amount < 0 => &commands.left,
                        _ => continue
                    };
                    execute_commands(commands, &x11_handler, debug_enabled);
                };
            }
            StateChanges::CrownTouched {modifiers} => {
                let modifiers = Modifier::from(modifiers);
                if let (_, Some(commands)) = config.get_mapping_for_modifiers(modifiers) {
                    execute_commands(&commands.touch, &x11_handler, debug_enabled);
                }
            }
            StateChanges::CrownReleased {modifiers} => {
                let modifiers = Modifier::from(modifiers);
                if let (_, Some(commands)) = config.get_mapping_for_modifiers(modifiers) {
                    execute_commands(&commands.release, &x11_handler, debug_enabled);
                }
            }
            StateChanges::CrownClicked { modifiers } => {
                let modifiers = Modifier::from(modifiers);
                if let (_, Some(commands)) = config.get_mapping_for_modifiers(modifiers) {
                    execute_commands(&commands.click, &x11_handler, debug_enabled);
                }
            }
            _ => {}
        }
    }
}
