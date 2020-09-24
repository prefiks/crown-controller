use std::collections::HashMap;
use std::fs::read_link;
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::thread::spawn;

use crossbeam_channel::{Receiver, Sender};
use mio::{Events, Interest, Poll, Token, Waker};
use mio::unix::SourceFd;
use x11rb::{atom_manager, CURRENT_TIME, NONE};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, change_window_attributes, ChangeWindowAttributesAux, EventMask,
                              get_keyboard_mapping, get_modifier_mapping, get_property,
                              KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
                              query_keymap};
use x11rb::protocol::xtest::fake_input;
use x11rb::rust_connection::RustConnection;

use super::StateChanges;

atom_manager! {
    pub AtomCollection: AtomCollectionCookie {
        _NET_WM_PID,
        _NET_ACTIVE_WINDOW,
    }
}
pub(crate) struct X11Handler {
    my_sender: Sender<X11Commands>,
    waker: Arc<Waker>,
}

impl X11Handler {
    pub fn new(event_receiver: Sender<StateChanges>) -> std::io::Result<X11Handler> {
        let (my_sender, my_receiver) = crossbeam_channel::unbounded();
        let poll = Poll::new()?;
        let waker = Arc::new(Waker::new(poll.registry(), Token(10))?);

        let _x = spawn(move || x11_listener(event_receiver, my_receiver, poll));

        Ok(X11Handler {
            my_sender,
            waker,
        })
    }

    pub fn send_key(&self, keysym: u32, modifiers: u8) {
        if self.my_sender.send(X11Commands::SendKey { keysym, modifiers }).is_ok() {
            let _ = self.waker.wake();
        }
    }
}

pub(crate) enum X11Commands {
    SendKey { keysym: u32, modifiers: u8 }
}

fn keysym_to_keycode_mapping(conn: &impl Connection) -> (HashMap<u32, (u8, u8)>, Vec<(u8, u8)>) {
    let setup = conn.setup();
    let reply = get_keyboard_mapping(conn, setup.min_keycode, setup.max_keycode - setup.min_keycode).
        unwrap().reply().unwrap();
    let mapping = reply.keysyms.chunks(reply.keysyms_per_keycode as usize)
        .enumerate().flat_map(|(index, keysyms)| {
        let keycode = index as u8 + setup.min_keycode;
        keysyms.iter().enumerate().map(move |(idx, keysym)| (*keysym, (keycode, idx as u8)))
    }).collect();

    let keycodes_of_mods = get_modifier_mapping(conn).
        map_or_else(|_| Vec::new(),
                    |c| c.reply().
                        map_or_else(|_| Vec::new(),
                                    |r| {
                                        let kpm = r.keycodes_per_modifier();
                                        let mut keycodes_of_mods = Vec::new();
                                        for (idx, keycodes) in r.keycodes.chunks_exact(kpm as usize).enumerate() {
                                            for keycode in keycodes {
                                                if *keycode == 0 { continue; }
                                                keycodes_of_mods.push((*keycode, 1 << idx as u8));
                                            }
                                        }
                                        keycodes_of_mods
                                    }));
    (mapping, keycodes_of_mods)
}

fn send_keypress(conn: &impl Connection, keycode: u8, modifiers: u8, keycodes_of_mods: &[(u8, u8)]) -> () {
    let mods_to_restore = query_keymap(conn).
        map_or_else(|_| Vec::new(),
                    |c| c.reply().
                        map_or_else(|_| Vec::new(),
                                    |r| {
                                        let mut to_restore = Vec::new();
                                        let mut pressed_modifiers = 0u8;
                                        for (mod_keycode, modifier) in keycodes_of_mods {
                                            if r.keys[(*mod_keycode / 8) as usize] & (1 << (*mod_keycode & 7)) != 0 {
                                                if *modifier & modifiers == 0 {
                                                    to_restore.push((*mod_keycode, KEY_PRESS_EVENT));
                                                }
                                                pressed_modifiers = pressed_modifiers | *modifier;
                                            }
                                        }
                                        let mut modifiers_to_press = modifiers & !pressed_modifiers;
                                        for (mod_keycode, modifier) in keycodes_of_mods {
                                            if *modifier & modifiers_to_press != 0 {
                                                let _ = fake_input(conn, KEY_PRESS_EVENT, *mod_keycode, CURRENT_TIME, NONE, 0, 0, 0);
                                                let _ = conn.flush();
                                                to_restore.push((*mod_keycode, KEY_RELEASE_EVENT));
                                                modifiers_to_press = modifiers_to_press & !*modifier;
                                            }
                                        }
                                        to_restore
                                    },
                        ));
    let _ = fake_input(conn, KEY_PRESS_EVENT, keycode, CURRENT_TIME, NONE, 0, 0, 0);
    let _ = conn.flush();
    let _ = fake_input(conn, KEY_RELEASE_EVENT, keycode, CURRENT_TIME, NONE, 0, 0, 0);
    let _ = conn.flush();
    for (mod_keycode, action) in mods_to_restore {
        let _ = fake_input(conn, action, mod_keycode, CURRENT_TIME, NONE, 0, 0, 0);
    }
    let _ = conn.flush();
}

fn x11_listener(sender: Sender<StateChanges>, receiver: Receiver<X11Commands>, mut poll: Poll) -> () {
    let mut events = Events::with_capacity(2);

    let (conn, screen_num) = RustConnection::connect(None).unwrap();
    let (mapping, keycodes_of_mods) = keysym_to_keycode_mapping(&conn);
    let screen = &conn.setup().roots[screen_num];
    let root_win = screen.root;
    let atoms = AtomCollection::new(&conn).unwrap().reply().unwrap();

    if change_window_attributes(&conn, root_win, &ChangeWindowAttributesAux::new().event_mask(EventMask::PropertyChange)).is_ok() {
        let _ = conn.flush();
    }

    let x11_token = Token(0);

    poll.registry().register(&mut SourceFd(&conn.stream().as_raw_fd()), x11_token, Interest::READABLE).unwrap();

    loop {
        let _ = poll.poll(&mut events, None);
        for event in &events {
            if event.token() != x11_token {
                if let Ok(command) = receiver.try_recv() {
                    match command {
                        X11Commands::SendKey { keysym, modifiers: key_modifiers } => {
                            if let Some((keycode, modifiers)) = mapping.get(&keysym) {
                                println!("command {:x?} {:x?} {:x?}, {:x?}", keycode, keysym, modifiers, key_modifiers);
                                send_keypress(&conn, *keycode, key_modifiers, &keycodes_of_mods);
                            }
                        }
                    }
                }
            } else {
                loop {
                    if let Ok(Some(event)) = conn.poll_for_event() {
                        match event {
                            Event::PropertyNotify(prop_notify) => {
                                if prop_notify.atom == atoms._NET_ACTIVE_WINDOW {
                                    let root_win = prop_notify.window;
                                    if let Some(win) =
                                    get_property(&conn, false, root_win, atoms._NET_ACTIVE_WINDOW,
                                                 AtomEnum::WINDOW, 0, 1).
                                        map_or(None, |p| p.reply().
                                            map_or(None, |r| r.value32().
                                                map_or(None, |mut v| v.next())))
                                    {
                                        if let Some(pid) =
                                        get_property(&conn, false, win, atoms._NET_WM_PID,
                                                     AtomEnum::CARDINAL, 0, 1).
                                            map_or(None, |v| v.reply().
                                                map_or(None, |r| r.value32().
                                                    map_or(None, |mut v| v.next())))
                                        {
                                            let program =
                                                if let Ok(path) = read_link(format!("/proc/{:}/exe", pid)) {
                                                    path.to_string_lossy().to_string()
                                                } else {
                                                    "".to_owned()
                                                };
                                            let _ = sender.send(StateChanges::FocusChanged { pid, program });
                                        } else {
                                            let _ = sender.send(StateChanges::FocusChanged { pid: 0, program: "".to_owned() });
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    } else {
                        break;
                    }
                }
            }
        }
    }
}

