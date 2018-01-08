// Copyright 2017 Mateusz Sieczko and other GilRs Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use super::FfDevice;
use ev::state::AxisInfo;
use ev::{Axis, Button, Event, EventType, NativeEvCode};
use gamepad::{self, GamepadImplExt, PowerInfo, Status};

use uuid::Uuid;
use winapi::winerror::{ERROR_DEVICE_NOT_CONNECTED, ERROR_SUCCESS};
use winapi::xinput::{self as xi, XINPUT_BATTERY_INFORMATION as XBatteryInfo,
                     XINPUT_GAMEPAD as XGamepad, XINPUT_STATE as XState, XINPUT_GAMEPAD_A,
                     XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK, XINPUT_GAMEPAD_DPAD_DOWN,
                     XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
                     XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB,
                     XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB,
                     XINPUT_GAMEPAD_START, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y};
use xinput;

use std::{mem, thread, i16, u16, u32, u8};
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

// Chosen by dice roll ;)
const EVENT_THREAD_SLEEP_TIME: u64 = 10;
const ITERATIONS_TO_CHECK_IF_CONNECTED: u64 = 100;

#[derive(Debug)]
pub struct Gilrs {
    gamepads: [gamepad::Gamepad; 4],
    rx: Receiver<Event>,
    not_observed: gamepad::Gamepad,
    additional_events: VecDeque<Event>,
}

impl Gilrs {
    pub fn new() -> Self {
        let gamepads = [
            gamepad_new(0),
            gamepad_new(1),
            gamepad_new(2),
            gamepad_new(3),
        ];

        let connected = [
            gamepads[0].is_connected(),
            gamepads[1].is_connected(),
            gamepads[2].is_connected(),
            gamepads[3].is_connected(),
        ];

        let additional_events = connected
            .iter()
            .enumerate()
            .filter(|&(_, &con)| con)
            .map(|(i, _)| Event::new(i, EventType::Connected))
            .collect();

        unsafe { xinput::XInputEnable(1) };
        let (tx, rx) = mpsc::channel();
        Self::spawn_thread(tx, connected);
        Gilrs {
            gamepads,
            rx,
            not_observed: gamepad::Gamepad::from_inner_status(Gamepad::none(), Status::NotObserved),
            additional_events,
        }
    }

    pub fn next_event(&mut self) -> Option<Event> {
        if let Some(event) = self.additional_events.pop_front() {
            Some(event)
        } else {
            self.rx.try_recv().ok()
        }
    }

    pub fn gamepad(&self, id: usize) -> &gamepad::Gamepad {
        self.gamepads.get(id).unwrap_or(&self.not_observed)
    }

    pub fn gamepad_mut(&mut self, id: usize) -> &mut gamepad::Gamepad {
        self.gamepads.get_mut(id).unwrap_or(&mut self.not_observed)
    }

    pub fn last_gamepad_hint(&self) -> usize {
        self.gamepads.len()
    }

    fn spawn_thread(tx: Sender<Event>, connected: [bool; 4]) {
        thread::spawn(move || unsafe {
            let mut prev_state = mem::zeroed::<XState>();
            let mut state = mem::zeroed::<XState>();
            let mut connected = connected;
            let mut counter = 0;

            loop {
                for id in 0..4 {
                    if *connected.get_unchecked(id)
                        || counter % ITERATIONS_TO_CHECK_IF_CONNECTED == 0
                    {
                        let val = xinput::XInputGetState(id as u32, &mut state);

                        if val == ERROR_SUCCESS {
                            if !connected.get_unchecked(id) {
                                *connected.get_unchecked_mut(id) = true;
                                let _ = tx.send(Event::new(id, EventType::Connected));
                            }

                            if state.dwPacketNumber != prev_state.dwPacketNumber {
                                Self::compare_state(id, &state.Gamepad, &prev_state.Gamepad, &tx);
                                prev_state = state;
                            }
                        } else if val == ERROR_DEVICE_NOT_CONNECTED && *connected.get_unchecked(id)
                        {
                            *connected.get_unchecked_mut(id) = false;
                            let _ = tx.send(Event::new(id, EventType::Disconnected));
                        }
                    }
                }

                counter = counter.wrapping_add(1);
                thread::sleep(Duration::from_millis(EVENT_THREAD_SLEEP_TIME));
            }
        });
    }

    fn compare_state(id: usize, g: &XGamepad, pg: &XGamepad, tx: &Sender<Event>) {
        fn normalize(val: i16) -> f32 {
            val as f32 / if val < 0 { -(i16::MIN as i32) } else { i16::MAX as i32 } as f32
        }

        if g.bLeftTrigger != pg.bLeftTrigger {
            let _ = tx.send(Event::new(
                id,
                EventType::AxisChanged(
                    Axis::LeftTrigger2,
                    g.bLeftTrigger as f32 / u8::MAX as f32,
                    native_ev_codes::AXIS_LT2,
                ),
            ));
        }
        if g.bRightTrigger != pg.bRightTrigger {
            let _ = tx.send(Event::new(
                id,
                EventType::AxisChanged(
                    Axis::RightTrigger2,
                    g.bRightTrigger as f32 / u8::MAX as f32,
                    native_ev_codes::AXIS_RT2,
                ),
            ));
        }
        if g.sThumbLX != pg.sThumbLX {
            let _ = tx.send(Event::new(
                id,
                EventType::AxisChanged(
                    Axis::LeftStickX,
                    normalize(g.sThumbLX),
                    native_ev_codes::AXIS_LSTICKX,
                ),
            ));
        }
        if g.sThumbLY != pg.sThumbLY {
            let _ = tx.send(Event::new(
                id,
                EventType::AxisChanged(
                    Axis::LeftStickY,
                    normalize(g.sThumbLY),
                    native_ev_codes::AXIS_LSTICKY,
                ),
            ));
        }
        if g.sThumbRX != pg.sThumbRX {
            let _ = tx.send(Event::new(
                id,
                EventType::AxisChanged(
                    Axis::RightStickX,
                    normalize(g.sThumbRX),
                    native_ev_codes::AXIS_RSTICKX,
                ),
            ));
        }
        if g.sThumbRY != pg.sThumbRY {
            let _ = tx.send(Event::new(
                id,
                EventType::AxisChanged(
                    Axis::RightStickY,
                    normalize(g.sThumbRY),
                    native_ev_codes::AXIS_RSTICKY,
                ),
            ));
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_UP) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_UP != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::DPadUp, native_ev_codes::BTN_DPAD_UP),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::DPadUp, native_ev_codes::BTN_DPAD_UP),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_DOWN) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_DOWN != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::DPadDown, native_ev_codes::BTN_DPAD_DOWN),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::DPadDown, native_ev_codes::BTN_DPAD_DOWN),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_LEFT) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_LEFT != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::DPadLeft, native_ev_codes::BTN_DPAD_LEFT),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::DPadLeft, native_ev_codes::BTN_DPAD_LEFT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_DPAD_RIGHT) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_DPAD_RIGHT != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::DPadRight, native_ev_codes::BTN_DPAD_RIGHT),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::DPadRight, native_ev_codes::BTN_DPAD_RIGHT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_START) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_START != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::Start, native_ev_codes::BTN_START),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::Start, native_ev_codes::BTN_START),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_BACK) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_BACK != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::Select, native_ev_codes::BTN_SELECT),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::Select, native_ev_codes::BTN_SELECT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_LEFT_THUMB) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_LEFT_THUMB != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::LeftThumb, native_ev_codes::BTN_LTHUMB),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::LeftThumb, native_ev_codes::BTN_LTHUMB),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_RIGHT_THUMB) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_RIGHT_THUMB != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::RightThumb, native_ev_codes::BTN_RTHUMB),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::RightThumb, native_ev_codes::BTN_RTHUMB),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_LEFT_SHOULDER) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_LEFT_SHOULDER != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::LeftTrigger, native_ev_codes::BTN_LT),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::LeftTrigger, native_ev_codes::BTN_LT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_RIGHT_SHOULDER) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_RIGHT_SHOULDER != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::RightTrigger, native_ev_codes::BTN_RT),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::RightTrigger, native_ev_codes::BTN_RT),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_A) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_A != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::South, native_ev_codes::BTN_SOUTH),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::South, native_ev_codes::BTN_SOUTH),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_B) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_B != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::East, native_ev_codes::BTN_EAST),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::East, native_ev_codes::BTN_EAST),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_X) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_X != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::West, native_ev_codes::BTN_WEST),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::West, native_ev_codes::BTN_WEST),
                )),
            };
        }
        if !is_mask_eq(g.wButtons, pg.wButtons, XINPUT_GAMEPAD_Y) {
            let _ = match g.wButtons & XINPUT_GAMEPAD_Y != 0 {
                true => tx.send(Event::new(
                    id,
                    EventType::ButtonPressed(Button::North, native_ev_codes::BTN_NORTH),
                )),
                false => tx.send(Event::new(
                    id,
                    EventType::ButtonReleased(Button::North, native_ev_codes::BTN_NORTH),
                )),
            };
        }
    }
}

#[derive(Debug)]
pub struct Gamepad {
    uuid: Uuid,
    id: u32,
}

impl Gamepad {
    fn none() -> Self {
        Gamepad {
            uuid: Uuid::nil(),
            id: u32::MAX,
        }
    }

    pub fn name(&self) -> &str {
        "Xbox Controller"
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn power_info(&self) -> PowerInfo {
        unsafe {
            let mut binfo = mem::uninitialized::<XBatteryInfo>();
            if xinput::XInputGetBatteryInformation(self.id, xi::BATTERY_DEVTYPE_GAMEPAD, &mut binfo)
                == ERROR_SUCCESS
            {
                match binfo.BatteryType {
                    xi::BATTERY_TYPE_WIRED => PowerInfo::Wired,
                    xi::BATTERY_TYPE_ALKALINE | xi::BATTERY_TYPE_NIMH => {
                        let lvl = match binfo.BatteryLevel {
                            xi::BATTERY_LEVEL_EMPTY => 0,
                            xi::BATTERY_LEVEL_LOW => 33,
                            xi::BATTERY_LEVEL_MEDIUM => 67,
                            xi::BATTERY_LEVEL_FULL => 100,
                            _ => unreachable!(),
                        };
                        if lvl == 100 {
                            PowerInfo::Charged
                        } else {
                            PowerInfo::Discharging(lvl)
                        }
                    }
                    _ => PowerInfo::Unknown,
                }
            } else {
                PowerInfo::Unknown
            }
        }
    }

    pub fn is_ff_supported(&self) -> bool {
        true
    }

    pub fn ff_device(&self) -> Option<FfDevice> {
        Some(FfDevice::new(self.id))
    }

    pub fn buttons(&self) -> &[NativeEvCode] {
        &native_ev_codes::BUTTONS
    }

    pub fn axes(&self) -> &[NativeEvCode] {
        &native_ev_codes::AXES
    }

    pub(crate) fn axis_info(&self, nec: NativeEvCode) -> Option<&AxisInfo> {
        native_ev_codes::AXES_INFO
            .get(nec as usize)
            .and_then(|o| o.as_ref())
    }
}

#[inline(always)]
fn is_mask_eq(l: u16, r: u16, mask: u16) -> bool {
    (l & mask != 0) == (r & mask != 0)
}

fn gamepad_new(id: u32) -> gamepad::Gamepad {
    let gamepad = Gamepad {
        uuid: Uuid::nil(),
        id,
    };

    let status = unsafe {
        let mut state = mem::zeroed::<XState>();
        if xinput::XInputGetState(id, &mut state) == ERROR_SUCCESS {
            Status::Connected
        } else {
            Status::NotObserved
        }
    };

    gamepad::Gamepad::from_inner_status(gamepad, status)
}

pub mod native_ev_codes {
    use std::i16::{MAX as I16_MAX, MIN as I16_MIN};
    use std::u8::{MAX as U8_MAX, MIN as U8_MIN};

    use winapi::xinput::{XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE, XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE,
                         XINPUT_GAMEPAD_TRIGGER_THRESHOLD};

    use ev::state::AxisInfo;

    pub const BTN_SOUTH: u16 = 0;
    pub const BTN_EAST: u16 = 1;
    pub const BTN_C: u16 = 2;
    pub const BTN_NORTH: u16 = 3;
    pub const BTN_WEST: u16 = 4;
    pub const BTN_Z: u16 = 5;
    pub const BTN_LT: u16 = 6;
    pub const BTN_RT: u16 = 7;
    pub const BTN_LT2: u16 = 8;
    pub const BTN_RT2: u16 = 9;
    pub const BTN_SELECT: u16 = 10;
    pub const BTN_START: u16 = 11;
    pub const BTN_MODE: u16 = 12;
    pub const BTN_LTHUMB: u16 = 13;
    pub const BTN_RTHUMB: u16 = 14;

    pub const BTN_DPAD_UP: u16 = 15;
    pub const BTN_DPAD_DOWN: u16 = 16;
    pub const BTN_DPAD_LEFT: u16 = 17;
    pub const BTN_DPAD_RIGHT: u16 = 18;

    pub const AXIS_LSTICKX: u16 = 0;
    pub const AXIS_LSTICKY: u16 = 1;
    pub const AXIS_LEFTZ: u16 = 2;
    pub const AXIS_RSTICKX: u16 = 3;
    pub const AXIS_RSTICKY: u16 = 4;
    pub const AXIS_RIGHTZ: u16 = 5;
    pub const AXIS_DPADX: u16 = 6;
    pub const AXIS_DPADY: u16 = 7;
    pub const AXIS_RT: u16 = 8;
    pub const AXIS_LT: u16 = 9;
    pub const AXIS_RT2: u16 = 10;
    pub const AXIS_LT2: u16 = 11;

    pub(super) static BUTTONS: [u16; 15] = [
        BTN_SOUTH, BTN_EAST, BTN_NORTH, BTN_WEST, BTN_LT, BTN_RT, BTN_SELECT, BTN_START, BTN_MODE,
        BTN_LTHUMB, BTN_RTHUMB, BTN_DPAD_UP, BTN_DPAD_DOWN, BTN_DPAD_LEFT, BTN_DPAD_RIGHT,
    ];

    pub(super) static AXES: [u16; 6] = [
        AXIS_LSTICKX,
        AXIS_LSTICKY,
        AXIS_RSTICKX,
        AXIS_RSTICKY,
        AXIS_RT2,
        AXIS_LT2,
    ];

    pub(super) static AXES_INFO: [Option<AxisInfo>; 12] = [
        // LeftStickX
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE as u32,
        }),
        // LeftStickY
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE as u32,
        }),
        // LeftZ
        None,
        // RightStickX
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE as u32,
        }),
        // RightStickY
        Some(AxisInfo {
            min: I16_MIN as i32,
            max: I16_MAX as i32,
            deadzone: XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE as u32,
        }),
        // RightZ
        None,
        // DPadX
        None,
        // DPadY
        None,
        // RightTrigger
        None,
        // LeftTrigger
        None,
        // RightTrigger2
        Some(AxisInfo {
            min: U8_MIN as i32,
            max: U8_MAX as i32,
            deadzone: XINPUT_GAMEPAD_TRIGGER_THRESHOLD as u32,
        }),
        // LeftTrigger2
        Some(AxisInfo {
            min: U8_MIN as i32,
            max: U8_MAX as i32,
            deadzone: XINPUT_GAMEPAD_TRIGGER_THRESHOLD as u32,
        }),
    ];
}
