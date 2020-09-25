use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::thread::spawn;

use crossbeam_channel::{Receiver, Sender};
use libc;
use mio::{Events, Interest, Poll, Token, Waker};
use mio::unix::SourceFd;

use crate::StateChanges;

#[derive(Debug)]
enum CrownCommands {
    EnableRatchet,
    DisableRatchet,
}

pub(crate) struct HidHandler {
    my_sender: Sender<CrownCommands>,
    waker: Arc<Waker>,
}

impl HidHandler {
    pub fn new(sender: Sender<StateChanges>, debug_enabled: bool) -> std::io::Result<HidHandler> {
        let (my_sender, my_receiver) = crossbeam_channel::unbounded();
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), Token(10))?);
        let _x = spawn(move || hid_listener(sender, my_receiver, poll, debug_enabled));

        Ok(HidHandler {
            my_sender,
            waker,
        })
    }

    pub fn enable_ratcher(&self) {
        if self.my_sender.send(CrownCommands::EnableRatchet).is_ok() {
            let _ = self.waker.wake();
        }
    }

    pub fn disable_ratcher(&self) {
        if self.my_sender.send(CrownCommands::DisableRatchet).is_ok() {
            let _ = self.waker.wake();
        }
    }
}

#[derive(Debug)]
pub(crate) enum CrownEvent {
    Connected,
    Touch,
    Leave,
    Press,
    Release,
    Rotate { amount: i16, pressed: bool, notch_amount: i16 },
    KeyPress { modifiers: u8 },
    Unknown,
}

fn decode_event(data: &[u8]) -> CrownEvent {
    match data {
        [0x11, _, 0x12, 0x00, rot, rot_am, rot_notch, _, _, _, pres, ..] if *rot != 0 => {
            CrownEvent::Rotate {
                amount: *rot_am as i8 as i16,
                pressed: *pres != 0x0,
                notch_amount: *rot_notch as i8 as i16,
            }
        }
        [0x11, _, 0x12, 0x00, 0x00, 0x00, 0x00, _, _, _, 0x01, ..] => CrownEvent::Press,
        [0x11, _, 0x12, 0x00, 0x00, 0x00, 0x00, _, _, _, 0x05, ..] => CrownEvent::Release,
        [0x11, _, 0x12, 0x00, 0x00, 0x00, 0x00, _, 0x01, ..] => CrownEvent::Touch,
        [0x11, _, 0x12, 0x00, 0x00, 0x00, 0x00, _, 0x03, ..] => CrownEvent::Leave,
        [0x20, _, 0x01, m, ..] => CrownEvent::KeyPress { modifiers: *m },
        [0x01, m, ..] => CrownEvent::KeyPress { modifiers: *m },
        [0x10, _, 0x41, ..] => CrownEvent::Connected,
        _ => CrownEvent::Unknown
    }
}

fn switch_ratcher(handle: &mut File, enabled: bool) -> () {
    if enabled {
        let _ = handle.write_all(&[0x11, 0x03, 0x12, 0x21, 0x02, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    } else {
        let _ = handle.write_all(&[0x11, 0x03, 0x12, 0x2a, 0x02, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
}


fn hid_listener(sender: Sender<StateChanges>, receiver: Receiver<CrownCommands>, mut poll: Poll,
                debug_enabled: bool) -> ()
{
    let mut ratchet_enabled = true;
    let mut modifiers = 0;
    let mut had_rotation = false;

    if let Ok(Some(dev_path)) = crate::udev::find_hidraw_device(0x46D, 0x4066) {
        let mut fh = OpenOptions::new().
            read(true).
            write(true).
            custom_flags(libc::O_NONBLOCK).
            open(dev_path).unwrap();

        let hidraw_token = Token(0);
        let mut events = Events::with_capacity(2);

        poll.registry().register(&mut SourceFd(&fh.as_raw_fd()), hidraw_token, Interest::READABLE).unwrap();

        let mut buf = [0u8; 1000];
        switch_ratcher(&mut fh, true);

        loop {
            let _ = poll.poll(&mut events, None);
            for event in &events {
                if event.token() != hidraw_token {
                    if let Ok(command) = receiver.try_recv() {
                        if debug_enabled {
                            println!("Mode events: {:?}", command);
                        }
                        match command {
                            CrownCommands::EnableRatchet => {
                                ratchet_enabled = true;
                                switch_ratcher(&mut fh, true);
                            }
                            CrownCommands::DisableRatchet => {
                                ratchet_enabled = false;
                                switch_ratcher(&mut fh, false);
                            }
                        }
                    }
                } else {
                    while let Ok(size) = fh.read(buf.as_mut()) {
                        let slice = &buf[0..size];
                        let event = decode_event(slice);
                        if debug_enabled {
                            println!("Crown events: {:x?} {:?}", slice, event);
                        }
                        match event {
                            CrownEvent::Connected => {
                                switch_ratcher(&mut fh, ratchet_enabled);
                            }
                            CrownEvent::KeyPress { modifiers: m } => {
                                modifiers = m;
                            }
                            CrownEvent::Touch => {
                                let _ = sender.send(StateChanges::CrownTouched { modifiers });
                            }
                            CrownEvent::Leave => {
                                let _ = sender.send(StateChanges::CrownReleased { modifiers });
                            }
                            CrownEvent::Press => {
                                had_rotation = false;
                            }
                            CrownEvent::Release if !had_rotation => {
                                let _ = sender.send(StateChanges::CrownClicked { modifiers });
                            }
                            CrownEvent::Rotate { notch_amount, amount, pressed } => {
                                if (!ratchet_enabled && amount != 0) || notch_amount != 0 {
                                    had_rotation = true;
                                }
                                if amount != 0 && (notch_amount != 0 || !ratchet_enabled) {
                                    let _ = sender.send(StateChanges::CrownRotated { modifiers, amount, notch_amount, pressed });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}
