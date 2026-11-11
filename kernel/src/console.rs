//! Kernel console: fans out to the Xen console and the UART.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{serial, xen};

static LOCK: AtomicBool = AtomicBool::new(false);

struct Sink;

impl Write for Sink {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        xen::console_write(s.as_bytes());
        serial::write(s.as_bytes());
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    while LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let _ = Sink.write_fmt(args);
    LOCK.store(false, Ordering::Release);
}

/// For fatal paths (panic, unexpected trap) that may fire while the console
/// lock is held: garbled output beats a silent deadlock.
pub fn print_unlocked(args: fmt::Arguments) {
    let _ = Sink.write_fmt(args);
}

macro_rules! kprintln {
    ($($arg:tt)*) => {
        $crate::console::print(format_args!("{}\n", format_args!($($arg)*)))
    };
}
