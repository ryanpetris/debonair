//! Turns Client input into an ordered queue of reports for the connected Target.
//!
//! Reports leave in the order input arrived, so a modifier pressed before a click reaches the
//! Target before the click. While a Target cannot keep up, consecutive pointer motion with the
//! same buttons is merged into one report. Keyboard, button and consumer reports carry complete
//! state, so when the queue is full the oldest report that a later report of its kind replaces is
//! dropped, and the last state of each kind always reaches the Target.
use crate::hid::{CONSUMER_MAX, CONSUMER_SLOTS, HIGH_RESOLUTION, Report};
use alloc::collections::VecDeque;

/// Reports waiting for the Target.
pub const QUEUE: usize = 64;
const LIMIT: i32 = i16::MAX as i32;
/// The most motion one input message can add, so a corrupt or extreme message cannot flood the
/// queue with split reports.
const MOTION_LIMIT: i32 = LIMIT * 4;

/// Error Roll-Over: more keys are held than a report can carry.
const ROLL_OVER: u8 = 0x01;

pub struct Reports {
    queue: VecDeque<Report>,
    modifiers: u8,
    keys: [u8; 6],
    buttons: u8,
    consumer: [u16; CONSUMER_SLOTS],
    wheel: Scroll,
    pan: Scroll,
}

/// Converts 1/120-detent scrolling to the Target's resolution.
#[derive(Default)]
struct Scroll {
    high_resolution: bool,
    remainder: i32,
}
impl Scroll {
    fn take(&mut self, units: i32) -> i32 {
        if self.high_resolution {
            return units;
        }
        let total = self.remainder.saturating_add(units);
        let detents = total / HIGH_RESOLUTION;
        self.remainder = total - detents * HIGH_RESOLUTION;
        detents
    }
}

impl Default for Reports {
    fn default() -> Self {
        Self::new()
    }
}

impl Reports {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            modifiers: 0,
            keys: [0; 6],
            buttons: 0,
            consumer: [0; CONSUMER_SLOTS],
            wheel: Scroll::default(),
            pan: Scroll::default(),
        }
    }

    /// Forgets queued reports and held state, as when a Target connects or disconnects.
    pub fn reset(&mut self) {
        self.queue.clear();
        self.modifiers = 0;
        self.keys = [0; 6];
        self.buttons = 0;
        self.consumer = [0; CONSUMER_SLOTS];
        self.wheel = Scroll::default();
        self.pan = Scroll::default();
    }

    /// Sets whether the Target reads wheel and pan in 1/120 of a detent.
    pub fn set_high_resolution(&mut self, wheel: bool, pan: bool) {
        if self.wheel.high_resolution != wheel {
            self.wheel = Scroll {
                high_resolution: wheel,
                remainder: 0,
            };
        }
        if self.pan.high_resolution != pan {
            self.pan = Scroll {
                high_resolution: pan,
                remainder: 0,
            };
        }
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn front(&self) -> Option<&Report> {
        self.queue.front()
    }
    pub fn pop(&mut self) -> Option<Report> {
        self.queue.pop_front()
    }

    fn push(&mut self, report: Report) {
        if self.queue.len() >= QUEUE {
            // Drop the oldest report that a later one of its kind replaces. A full queue always
            // has one: it holds more reports than there are kinds.
            let kind = |r: &Report| core::mem::discriminant(r);
            let replaced = (0..self.queue.len()).find(|&i| {
                self.queue
                    .iter()
                    .skip(i + 1)
                    .chain(core::iter::once(&report))
                    .any(|later| kind(later) == kind(&self.queue[i]))
            });
            self.queue.remove(replaced.unwrap_or(0));
        }
        if self.queue.try_reserve(1).is_ok() {
            self.queue.push_back(report);
        }
    }

    /// Records the complete keyboard state. Modifier usages in `keys` count as modifiers.
    pub fn keyboard(&mut self, modifiers: u8, keys: &[u8]) {
        let mut modifiers = modifiers;
        let mut held = [0u8; 6];
        let mut count = 0;
        let mut overflow = false;
        for &key in keys {
            match key {
                0xe0..=0xe7 => modifiers |= 1 << (key - 0xe0),
                0x04..=0xdf if !held[..count.min(6)].contains(&key) => {
                    if count == 6 {
                        overflow = true;
                    } else {
                        held[count] = key;
                        count += 1;
                    }
                }
                _ => {}
            }
        }
        if overflow {
            held = [ROLL_OVER; 6];
        }
        if (modifiers, held) == (self.modifiers, self.keys) {
            return;
        }
        self.modifiers = modifiers;
        self.keys = held;
        self.push(Report::Keyboard {
            modifiers,
            keys: held,
        });
    }

    /// Records held buttons and adds relative motion and scrolling in 1/120 of a detent.
    pub fn mouse(&mut self, buttons: u8, x: i32, y: i32, wheel: i32, pan: i32) {
        let buttons = buttons & 0x1f;
        let bound = |value: i32| value.clamp(-MOTION_LIMIT, MOTION_LIMIT);
        let (x, y) = (bound(x), bound(y));
        let wheel = bound(self.wheel.take(bound(wheel)));
        let pan = bound(self.pan.take(bound(pan)));
        let (mut x, mut y, mut wheel, mut pan) = (x, y, wheel, pan);
        let mut changed = buttons != self.buttons;
        self.buttons = buttons;
        // Large motion is split across reports rather than clamped.
        loop {
            if !changed && x == 0 && y == 0 && wheel == 0 && pan == 0 {
                return;
            }
            if !changed
                && let Some(Report::Mouse {
                    buttons: last,
                    x: lx,
                    y: ly,
                    wheel: lw,
                    pan: lp,
                }) = self.queue.back_mut()
                && *last == buttons
            {
                let take = |held: &mut i16, value: &mut i32| {
                    let sum = i32::from(*held).saturating_add(*value).clamp(-LIMIT, LIMIT);
                    *value -= sum - i32::from(*held);
                    *held = sum as i16;
                };
                take(lx, &mut x);
                take(ly, &mut y);
                take(lw, &mut wheel);
                take(lp, &mut pan);
                if x == 0 && y == 0 && wheel == 0 && pan == 0 {
                    return;
                }
            }
            let part = |value: &mut i32| {
                let part = (*value).clamp(-LIMIT, LIMIT);
                *value -= part;
                part as i16
            };
            let report = Report::Mouse {
                buttons,
                x: part(&mut x),
                y: part(&mut y),
                wheel: part(&mut wheel),
                pan: part(&mut pan),
            };
            self.push(report);
            changed = false;
        }
    }

    /// Records the complete set of held consumer controls.
    pub fn consumer(&mut self, usages: &[u32]) {
        let mut held = [0u16; CONSUMER_SLOTS];
        let mut count = 0;
        for &usage in usages {
            if count == CONSUMER_SLOTS {
                break;
            }
            if let Ok(usage) = u16::try_from(usage)
                && (1..=CONSUMER_MAX).contains(&usage)
                && !held[..count].contains(&usage)
            {
                held[count] = usage;
                count += 1;
            }
        }
        if held == self.consumer {
            return;
        }
        self.consumer = held;
        self.push(Report::Consumer(held));
    }

    /// Releases every held key, button and consumer control.
    pub fn release_all(&mut self) {
        self.keyboard(0, &[]);
        self.mouse(0, 0, 0, 0, 0);
        self.consumer(&[]);
    }

    /// Whether anything is held.
    pub fn holding(&self) -> bool {
        self.modifiers != 0
            || self.keys != [0; 6]
            || self.buttons != 0
            || self.consumer != [0; CONSUMER_SLOTS]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn drain(reports: &mut Reports) -> Vec<Report> {
        core::iter::from_fn(|| reports.pop()).collect()
    }
    fn mouse(buttons: u8, x: i16, y: i16, wheel: i16, pan: i16) -> Report {
        Report::Mouse {
            buttons,
            x,
            y,
            wheel,
            pan,
        }
    }

    #[test]
    fn keyboard_reports_follow_input_order_with_motion() {
        let mut r = Reports::new();
        r.keyboard(0x01, &[]);
        r.mouse(0, 5, 0, 0, 0);
        r.mouse(1, 0, 0, 0, 0);
        r.mouse(1, 3, 4, 0, 0);
        r.mouse(1, 1, 1, 0, 0);
        assert_eq!(
            drain(&mut r),
            [
                Report::Keyboard {
                    modifiers: 1,
                    keys: [0; 6]
                },
                mouse(0, 5, 0, 0, 0),
                mouse(1, 4, 5, 0, 0),
            ]
        );
    }

    #[test]
    fn motion_does_not_merge_across_a_button_change() {
        let mut r = Reports::new();
        r.mouse(0, 1, 0, 0, 0);
        r.mouse(1, 0, 0, 0, 0);
        r.mouse(0, 2, 0, 0, 0);
        assert_eq!(
            drain(&mut r),
            [
                mouse(0, 1, 0, 0, 0),
                mouse(1, 0, 0, 0, 0),
                mouse(0, 2, 0, 0, 0)
            ]
        );
    }

    #[test]
    fn large_motion_splits_instead_of_clamping() {
        let mut r = Reports::new();
        r.mouse(0, 32000, 0, 0, 0);
        r.mouse(0, 1000, -70000, 0, 0);
        let reports = drain(&mut r);
        let sum = reports
            .iter()
            .fold((0i32, 0i32), |(x, y), report| match report {
                Report::Mouse { x: dx, y: dy, .. } => (x + i32::from(*dx), y + i32::from(*dy)),
                _ => unreachable!(),
            });
        assert_eq!(sum, (33000, -70000));
        for report in reports {
            let Report::Mouse { x, y, .. } = report else {
                unreachable!()
            };
            assert!(x > i16::MIN && y > i16::MIN);
        }
    }

    #[test]
    fn low_resolution_scrolling_accumulates_detents() {
        let mut r = Reports::new();
        r.mouse(0, 0, 0, 50, 0);
        assert!(r.is_empty());
        r.mouse(0, 0, 0, 80, -130);
        assert_eq!(drain(&mut r), [mouse(0, 0, 0, 1, -1)]);
        r.set_high_resolution(true, true);
        r.mouse(0, 0, 0, 7, -3);
        assert_eq!(drain(&mut r), [mouse(0, 0, 0, 7, -3)]);
    }

    #[test]
    fn keyboard_dedupes_and_rolls_over() {
        let mut r = Reports::new();
        r.keyboard(0, &[0x04, 0x04, 0xe1]);
        r.keyboard(0x02, &[0x04]);
        r.keyboard(0, &[4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(
            drain(&mut r),
            [
                Report::Keyboard {
                    modifiers: 0x02,
                    keys: [4, 0, 0, 0, 0, 0]
                },
                Report::Keyboard {
                    modifiers: 0,
                    keys: [ROLL_OVER; 6]
                },
            ]
        );
    }

    #[test]
    fn full_queue_keeps_the_latest_state() {
        let mut r = Reports::new();
        for i in 0..(QUEUE + 10) {
            r.keyboard(0, &[if i % 2 == 0 { 4 } else { 5 }]);
        }
        r.keyboard(0, &[]);
        assert_eq!(r.len(), QUEUE);
        let last = drain(&mut r).pop().unwrap();
        assert_eq!(
            last,
            Report::Keyboard {
                modifiers: 0,
                keys: [0; 6]
            }
        );
    }

    #[test]
    fn full_queue_keeps_a_release_behind_motion() {
        let mut r = Reports::new();
        r.keyboard(0, &[4]);
        r.keyboard(0, &[]);
        for i in 0..(QUEUE * 2) {
            r.mouse((i % 2) as u8, 1, 0, 0, 0);
        }
        let reports = drain(&mut r);
        assert_eq!(reports.len(), QUEUE);
        let keyboard: Vec<_> = reports
            .iter()
            .filter(|r| matches!(r, Report::Keyboard { .. }))
            .collect();
        assert_eq!(
            keyboard,
            [&Report::Keyboard {
                modifiers: 0,
                keys: [0; 6]
            }]
        );
    }

    #[test]
    fn extreme_motion_is_bounded() {
        let mut r = Reports::new();
        r.set_high_resolution(true, true);
        r.mouse(0, i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        r.mouse(0, i32::MAX, 0, 0, 0);
        assert!(r.len() <= 8);
        let total: i32 = drain(&mut r)
            .iter()
            .map(|report| match report {
                Report::Mouse { x, .. } => i32::from(*x),
                _ => 0,
            })
            .sum();
        assert_eq!(total, MOTION_LIMIT * 2);
    }

    #[test]
    fn release_all_clears_only_held_state() {
        let mut r = Reports::new();
        r.release_all();
        assert!(r.is_empty());
        r.consumer(&[0xe9, 0xe9, 0x10000, 0]);
        r.mouse(2, 0, 0, 0, 0);
        assert!(r.holding());
        drain(&mut r);
        r.release_all();
        assert!(!r.holding());
        assert_eq!(
            drain(&mut r),
            [mouse(0, 0, 0, 0, 0), Report::Consumer([0, 0])]
        );
    }
}
